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

/// Query structs must say so, utoipa otherwise declares their fields as path parameters, which
/// broke model generation for the app once.
#[test]
fn every_path_parameter_appears_in_its_route() {
    let doc: serde_json::Value = serde_json::from_str(&r16a_stream_backend::openapi::to_json().unwrap()).unwrap();
    let mut wrong = Vec::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            for p in op["parameters"].as_array().into_iter().flatten() {
                let name = p["name"].as_str().unwrap();
                if p["in"] == "path" && !path.contains(&format!("{{{name}}}")) {
                    wrong.push(format!("{method} {path} {name}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "path parameters without a placeholder: {wrong:?}");
}
