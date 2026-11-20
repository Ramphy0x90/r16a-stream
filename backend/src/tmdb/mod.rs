//! TMDB lookups that prefill a title draft for admins (SPEC 8.1).
//!
//! Nothing is written to the catalog here. The lookup returns a draft that the admin edits and
//! then saves through the normal admin routes. TMDB is an external source, so every string is
//! cleaned of control characters and cut to the column limits, and every number is range checked,
//! before it reaches the admin.

pub mod client;
pub mod draft;
pub mod handlers;
pub mod reference;
pub mod repository;
