//! Variants, uploads and transcode administration (SPEC 4.5, 7.1, 7.9, 7.10).
//!
//! The API starts multipart uploads, signs part URLs for the admin's browser, completes them
//! and queues the transcode. The worker does the rest. Only the API holds the uploads write key,
//! so it also deletes sources under the retention rules.

pub mod dto;
pub mod handlers;
pub mod repository;
pub mod service;
pub mod tasks;
