//! `docs/openapi.json` must match the handlers. Web and mobile clients are generated from it,
//! so drift would ship clients that disagree with the server. Refresh with
//! `R16A_WRITE_OPENAPI=1 cargo test --test openapi`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

#[test]
fn committed_openapi_document_is_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/openapi.json");
    let generated = r16a_stream_backend::openapi::to_json().unwrap();
    if std::env::var("R16A_WRITE_OPENAPI").as_deref() == Ok("1") {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/openapi.json is stale, run R16A_WRITE_OPENAPI=1 cargo test --test openapi"
    );
}
