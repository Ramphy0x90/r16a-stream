//! Cross-cutting HTTP pieces shared by every route.

pub mod client_ip;
pub mod cursor;
pub mod etag;
pub mod json;
pub mod path;
pub mod rate_limit;
pub mod request_id;
pub mod security_headers;
pub mod trace;
