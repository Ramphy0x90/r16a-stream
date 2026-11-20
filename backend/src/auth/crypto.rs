//! Randomness, hashing and sealing for the login flow and the web sessions.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand::{TryRng, rngs::SysRng};
use secrecy::{ExposeSecret, SecretBox};
use sha2::{Digest, Sha256};

use crate::{config::AuthConfig, error::AppError};

const NONCE_LEN: usize = 24;

/// Bytes straight from the operating system. Session cookies, state, nonces and PKCE verifiers
/// all come from here, never from a userspace generator that could be forked or reseeded.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], AppError> {
    let mut out = [0u8; N];
    SysRng
        .try_fill_bytes(&mut out)
        .map_err(|e| AppError::internal(format!("os random source failed {e}")))?;
    Ok(out)
}

/// 32 random bytes as base64url, 43 characters, for values that travel in URLs and cookies.
pub fn random_token() -> Result<String, AppError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn b64url(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// The S256 PKCE challenge for a verifier, RFC 7636 section 4.2.
pub fn pkce_challenge(verifier: &str) -> String {
    b64url(&sha256(verifier.as_bytes()))
}

/// Associated data of a session's sealed refresh token (DECISIONS 020). The label keeps the
/// two kinds of sealed value apart and the version lets the format change later.
pub fn session_aad(session_id: uuid::Uuid) -> Vec<u8> {
    format!("r16a-stream:session:v1:{session_id}:refresh_token").into_bytes()
}

/// Associated data of a login flow's sealed PKCE verifier.
pub fn flow_aad(state_hash: &[u8; 32]) -> Vec<u8> {
    format!("r16a-stream:flow:v1:{}:pkce_verifier", b64url(state_hash)).into_bytes()
}

/// One session key and the id stored next to everything it sealed.
struct Key {
    id: i16,
    cipher: XChaCha20Poly1305,
}

impl Key {
    fn new(secret: &SecretBox<[u8; 32]>) -> Self {
        let bytes = secret.expose_secret();
        Self {
            id: key_id(bytes),
            cipher: XChaCha20Poly1305::new(&(*bytes).into()),
        }
    }
}

/// The id is derived from the key, so it survives rotation without being configured and a row
/// always names the key that sealed it.
fn key_id(key: &[u8; 32]) -> i16 {
    let h = sha256(key);
    i16::from_be_bytes([h[0], h[1]])
}

/// XChaCha20-Poly1305 with the current session key, and the previous one for opening only.
///
/// The associated data binds each sealed value to the row it belongs to, see [`session_aad`]
/// and [`flow_aad`], so a ciphertext copied into another row fails to open.
pub struct SessionKeys {
    current: Key,
    previous: Option<Key>,
}

impl std::fmt::Debug for SessionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKeys([redacted])")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("R16A_SESSION_KEY_PREVIOUS has the same key id as R16A_SESSION_KEY, use a different key")]
    SameId,
}

impl SessionKeys {
    pub fn from_config(cfg: &AuthConfig) -> Result<Self, KeyError> {
        let current = Key::new(&cfg.session_key);
        let previous = cfg.session_key_previous.as_ref().map(Key::new);
        if previous.as_ref().is_some_and(|p| p.id == current.id) {
            return Err(KeyError::SameId);
        }
        Ok(Self { current, previous })
    }

    pub fn current_id(&self) -> i16 {
        self.current.id
    }

    /// Returns the nonce followed by the ciphertext, and the id of the key used.
    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<(Vec<u8>, i16), AppError> {
        let nonce_bytes = random_bytes::<NONCE_LEN>()?;
        let nonce = XNonce::from(nonce_bytes);
        let ct = self
            .current
            .cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad })
            .map_err(|_| AppError::internal("sealing failed"))?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok((out, self.current.id))
    }

    /// Opens a value sealed by the current or the previous key. `None` means the key is gone or
    /// the value was tampered with, which callers treat as a missing session.
    pub fn open(&self, sealed: &[u8], aad: &[u8], key_id: i16) -> Option<Vec<u8>> {
        let key = [Some(&self.current), self.previous.as_ref()]
            .into_iter()
            .flatten()
            .find(|k| k.id == key_id)?;
        if sealed.len() < NONCE_LEN {
            return None;
        }
        let (nonce, ct) = sealed.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).ok()?;
        key.cipher.decrypt(&nonce, Payload { msg: ct, aad }).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(current: u8, previous: Option<u8>) -> SessionKeys {
        let k = |b: u8| Key::new(&SecretBox::new(Box::new([b; 32])));
        SessionKeys {
            current: k(current),
            previous: previous.map(k),
        }
    }

    #[test]
    fn seal_and_open_round_trip() {
        let k = keys(1, None);
        let (sealed, id) = k.seal(b"refresh-token", b"session-a").expect("seal");
        assert_eq!(
            k.open(&sealed, b"session-a", id).as_deref(),
            Some(&b"refresh-token"[..])
        );
    }

    #[test]
    fn other_row_or_tampering_does_not_open() {
        let k = keys(1, None);
        let (mut sealed, id) = k.seal(b"refresh-token", b"session-a").expect("seal");
        assert!(k.open(&sealed, b"session-b", id).is_none(), "bound to its row");
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(k.open(&sealed, b"session-a", id).is_none(), "authenticated");
        assert!(k.open(&[0; 10], b"session-a", id).is_none(), "short input");
    }

    #[test]
    fn previous_key_still_opens_after_rotation() {
        let old = keys(1, None);
        let (sealed, id) = old.seal(b"t", b"a").expect("seal");
        let rotated = keys(2, Some(1));
        assert_eq!(rotated.open(&sealed, b"a", id).as_deref(), Some(&b"t"[..]));
        let dropped = keys(2, None);
        assert!(dropped.open(&sealed, b"a", id).is_none());
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_tokens_are_43_url_safe_characters() {
        let t = random_token().expect("token");
        assert_eq!(t.len(), 43);
        assert!(t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(t, random_token().expect("token"));
    }
}
