//! Request logging that never records a raw path or query.
//!
//! Playback tokens travel in the URL path and presigned query strings carry signatures, so the
//! span holds only the method and the matched route template, such as
//! `/api/v1/play/{token}/master.m3u8`. Requests that match no route log `unmatched`.

use std::time::Duration;

use axum::{
    extract::{MatchedPath, Request},
    response::Response,
};
use tower_http::{
    classify::{ServerErrorsAsFailures, SharedClassifier},
    trace::TraceLayer,
};
use tracing::Span;

use crate::http::request_id::RequestId;

type MakeSpan = fn(&Request) -> Span;
type OnResponse = fn(&Response, Duration, &Span);

pub fn layer() -> TraceLayer<SharedClassifier<ServerErrorsAsFailures>, MakeSpan, (), OnResponse> {
    TraceLayer::new_for_http()
        .make_span_with(make_span as MakeSpan)
        .on_request(())
        .on_response(on_response as OnResponse)
}

fn make_span(req: &Request) -> Span {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str);
    let request_id = req.extensions().get::<RequestId>().map_or("unknown", |r| r.0.as_str());
    tracing::info_span!("http", method = %req.method(), route = %route, request_id = %request_id)
}

fn on_response(res: &Response, latency: Duration, _span: &Span) {
    tracing::info!(
        status = res.status().as_u16(),
        latency_ms = latency.as_millis(),
        "response"
    );
}
