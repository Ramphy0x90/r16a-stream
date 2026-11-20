//! Session and flow cookies, and the `returnTo` rule.
//!
//! Both cookies carry the `__Host-` prefix with `Secure` and `Path=/`, so a sibling host cannot
//! plant or shadow them (SPEC 6.3, DECISIONS 015). `R16A_DEV_INSECURE_COOKIE` drops `Secure`
//! and with it the prefix, which browsers would otherwise refuse on plain HTTP. Config refuses
//! that switch with an https origin or in production.

use axum::http::{HeaderMap, HeaderValue, header};

const SESSION: &str = "r16a_stream_session";
const FLOW: &str = "r16a_stream_flow";

/// Idle lifetime of a web session, SPEC 6.3.
pub const SESSION_IDLE_SECS: i32 = 7 * 24 * 3600;
/// Absolute lifetime of a web session, equal to Authentik's refresh token validity.
pub const SESSION_ABSOLUTE_SECS: i32 = 30 * 24 * 3600;
/// Lifetime of an in-flight login, matching the `oidc_flows` row.
pub const FLOW_SECS: i32 = 600;

#[derive(Debug, Clone, Copy)]
pub struct CookiePolicy {
    pub insecure: bool,
}

impl CookiePolicy {
    fn name(self, base: &str) -> String {
        if self.insecure {
            base.to_owned()
        } else {
            format!("__Host-{base}")
        }
    }

    fn build(self, base: &str, value: &str, same_site: &str, max_age: i32) -> HeaderValue {
        let secure = if self.insecure { "" } else { "; Secure" };
        let s = format!(
            "{}={value}; Path=/; HttpOnly; SameSite={same_site}; Max-Age={max_age}{secure}",
            self.name(base)
        );
        // Values are base64url and names are constants, so this cannot fail. A failure would
        // only drop the cookie, never send a malformed header.
        HeaderValue::from_str(&s).unwrap_or_else(|_| HeaderValue::from_static(""))
    }

    /// `SameSite=Strict` keeps the cookie off every cross-site request, including top-level
    /// navigations, which is what makes the CSRF header check a second layer and not the only one.
    pub fn session(self, value: &str) -> HeaderValue {
        self.build(SESSION, value, "Strict", SESSION_ABSOLUTE_SECS)
    }

    pub fn clear_session(self) -> HeaderValue {
        self.build(SESSION, "", "Strict", 0)
    }

    /// `Lax`, because the callback arrives as a top-level navigation from Authentik, which is a
    /// cross-site request that a `Strict` cookie would not accompany.
    pub fn flow(self, value: &str) -> HeaderValue {
        self.build(FLOW, value, "Lax", FLOW_SECS)
    }

    pub fn clear_flow(self) -> HeaderValue {
        self.build(FLOW, "", "Lax", 0)
    }

    pub fn read_session(self, headers: &HeaderMap) -> Option<String> {
        read(headers, &self.name(SESSION))
    }

    pub fn read_flow(self, headers: &HeaderMap) -> Option<String> {
        read(headers, &self.name(FLOW))
    }
}

/// Finds a cookie by exact name. A value that is not a 43 character base64url token cannot be
/// one we issued and is ignored, which also keeps junk out of the hash lookups.
fn read(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
        .filter(|v| v.len() == 43 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
}

/// Accepts only a same-origin absolute path. Anything else becomes `/` (T-AUTH-2).
///
/// Refused are a missing leading slash, `//` and `/\` (both read as a host by browsers), any
/// backslash, control or whitespace character, a scheme, and values over 512 bytes. The path
/// is not decoded, so `%2F%2F` stays a harmless path segment.
pub fn sanitize_return_to(raw: Option<&str>) -> String {
    let Some(v) = raw else {
        return String::from("/");
    };
    let ok = v.len() <= 512
        && v.starts_with('/')
        && !v.starts_with("//")
        && !v.contains('\\')
        && !v.chars().any(|c| c.is_control() || c.is_whitespace())
        && !v.split(['?', '#']).next().unwrap_or_default().contains(':');
    if ok { v.to_owned() } else { String::from("/") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_accepts_only_same_origin_paths() {
        for good in ["/", "/title/123", "/search?q=a:b", "/lists#x", "/%2F%2Fevil.example"] {
            assert_eq!(sanitize_return_to(Some(good)), good, "{good}");
        }
        for bad in [
            "",
            "title",
            "//evil.example",
            "/\\evil.example",
            "\\\\evil.example",
            "https://evil.example",
            "/javascript:alert(1)",
            "/a\nb",
            "/a b",
            "/\t/evil.example",
        ] {
            assert_eq!(sanitize_return_to(Some(bad)), "/", "{bad:?}");
        }
        assert_eq!(sanitize_return_to(Some(&format!("/{}", "a".repeat(600)))), "/");
        assert_eq!(sanitize_return_to(None), "/");
    }

    #[test]
    fn secure_cookies_carry_the_host_prefix() {
        let p = CookiePolicy { insecure: false };
        let c = p.session("v").to_str().expect("ascii").to_owned();
        assert!(c.starts_with("__Host-r16a_stream_session=v; Path=/; HttpOnly; SameSite=Strict"));
        assert!(c.ends_with("; Secure"));
        let f = p.flow("v").to_str().expect("ascii").to_owned();
        assert!(f.contains("SameSite=Lax") && f.contains("Max-Age=600"));
        let dev = CookiePolicy { insecure: true }
            .session("v")
            .to_str()
            .expect("ascii")
            .to_owned();
        assert!(dev.starts_with("r16a_stream_session=v;") && !dev.contains("Secure"));
    }

    #[test]
    fn reads_only_well_formed_values_by_exact_name() {
        let token = "a".repeat(43);
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "x=1; r16a_stream_session=bad; __Host-r16a_stream_session={token}"
            ))
            .expect("header"),
        );
        let p = CookiePolicy { insecure: false };
        assert_eq!(p.read_session(&h), Some(token));
        assert_eq!(
            CookiePolicy { insecure: true }.read_session(&h),
            None,
            "malformed value ignored"
        );
        assert_eq!(p.read_flow(&h), None);
    }
}
