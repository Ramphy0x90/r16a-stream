//! Parses what an admin pastes into the TMDB lookup field.

use url::Url;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmdbKind {
    Movie,
    Tv,
}

impl TmdbKind {
    /// Path segment in TMDB URLs and API paths.
    pub fn segment(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

/// A TMDB title, by kind and numeric id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TmdbRef {
    pub kind: TmdbKind,
    pub id: u32,
}

/// Accepts a numeric id together with `kind` (`movie` or `show`), or a link of the form
/// `https://www.themoviedb.org/movie/<id>[-slug]` or `/tv/<id>[-slug]`. Query and fragment
/// of a link are ignored. Anything else, including other hosts, is 400, so the value can never
/// steer where the backend sends a request.
pub fn parse(raw: &str, kind: Option<&str>) -> Result<TmdbRef, AppError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 300 {
        return Err(AppError::BadRequest("invalid-tmdb-ref"));
    }
    if raw.bytes().all(|b| b.is_ascii_digit()) {
        let kind = match kind {
            Some("movie") => TmdbKind::Movie,
            Some("show") => TmdbKind::Tv,
            _ => return Err(AppError::BadRequest("invalid-tmdb-kind")),
        };
        return Ok(TmdbRef { kind, id: id(raw)? });
    }
    let url = Url::parse(raw).map_err(|_| AppError::BadRequest("invalid-tmdb-ref"))?;
    if url.scheme() != "https" || url.host_str() != Some("www.themoviedb.org") || url.port().is_some() {
        return Err(AppError::BadRequest("invalid-tmdb-ref"));
    }
    let segments: Vec<&str> = url
        .path_segments()
        .map_or_else(Vec::new, |s| s.filter(|p| !p.is_empty()).collect());
    let [section, slug] = segments.as_slice() else {
        return Err(AppError::BadRequest("invalid-tmdb-ref"));
    };
    let kind = match *section {
        "movie" => TmdbKind::Movie,
        "tv" => TmdbKind::Tv,
        _ => return Err(AppError::BadRequest("invalid-tmdb-ref")),
    };
    let digits = slug.split('-').next().unwrap_or_default();
    Ok(TmdbRef { kind, id: id(digits)? })
}

fn id(digits: &str) -> Result<u32, AppError> {
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::BadRequest("invalid-tmdb-ref"));
    }
    digits
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or(AppError::BadRequest("invalid-tmdb-ref"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ids_and_tmdb_links() {
        assert_eq!(
            parse("603", Some("movie")).expect("id"),
            TmdbRef {
                kind: TmdbKind::Movie,
                id: 603
            }
        );
        assert_eq!(parse("1396", Some("show")).expect("id").kind, TmdbKind::Tv);
        let r = parse("https://www.themoviedb.org/tv/1396-breaking-bad?language=it-IT#x", None).expect("link");
        assert_eq!((r.kind, r.id), (TmdbKind::Tv, 1396));
        let r = parse("https://www.themoviedb.org/movie/603/", None).expect("trailing slash");
        assert_eq!((r.kind, r.id), (TmdbKind::Movie, 603));
    }

    #[test]
    fn refuses_everything_else() {
        for bad in [
            "",
            "603",
            "http://www.themoviedb.org/movie/603",
            "https://themoviedb.org.evil.example/movie/603",
            "https://evil.example/movie/603",
            "https://www.themoviedb.org:8443/movie/603",
            "https://www.themoviedb.org/person/603",
            "https://www.themoviedb.org/movie/abc",
            "https://www.themoviedb.org/movie/603/images",
            "https://www.themoviedb.org/movie/0",
            "https://www.themoviedb.org/movie/99999999999",
            "../../etc/passwd",
        ] {
            assert!(parse(bad, None).is_err(), "{bad}");
        }
        assert!(parse("603", Some("person")).is_err());
    }
}
