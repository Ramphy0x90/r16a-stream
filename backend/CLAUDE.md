# backend

**Phase 3 done** (2026-11-20). Foundations, authentication, catalog, admin content and images. The rest in Phases 4 to 8.

The r16a-stream API and transcode worker. Rust 2024, axum 0.8, sqlx 0.9 on PostgreSQL 15, crate `r16a-stream-backend`, binaries `api` and `worker`. API, data model and flows are in `docs/SPEC.md` sections 4 to 10.

## Rules specific to this folder

- `#![forbid(unsafe_code)]`. Clippy `-D warnings` with `clippy::pedantic` on (noisy lints allowed explicitly with a reason), rustfmt `max_width = 120`, `cargo deny check` must pass.
- No `unwrap` or `expect` outside tests and startup. Errors are typed with `thiserror`.
- Handlers do HTTP only, `service.rs` holds the rules, `repository.rs` holds SQL only through `sqlx::query!` macros. `.sqlx/` is committed and refreshed after every query change.
- Request bodies use `ApiJson<T>` (`http/json.rs`), never axum's `Json`, so framework errors that echo input never reach the client. DTOs carry `#[serde(deny_unknown_fields)]` and length limits on every string.
- Errors are `AppError`, rendered as RFC 9457 problem+json with `type` `urn:r16a-stream:problem:<slug>` and `requestId`. `Internal` is logged, never shown. Never put secrets, URLs or input values into an error.
- Every query on user data filters on the caller. Not found and not owned both return 404. Admin routes take the `Admin` extractor, which answers 404 to non-admins.
- Logs carry the matched route template, never the raw path or query (playback tokens live in the path).
- Every outbound call has a timeout from config. Video bytes never pass through the API.
- Writes to Garage are retried with backoff. A storage failure in the worker requeues the job, it never fails it. In the API, a route that writes to Garage answers `503 storage_degraded` while the write probe fails. The API's `/readyz` checks only the database.
- The worker runs as DB role `r16a_stream_worker` with column-level grants (SPEC 4.9). A query in worker code that touches a table or column outside those grants fails at runtime, and `tests/db_roles.rs` must be extended whenever grants change.
- Source objects are deleted only by the API's retention task, never by the worker, and only after the attempt is verified (SPEC 7.9).
- ffmpeg and ffprobe are spawned with argument arrays, `env_clear()`, `-protocol_whitelist file` and a `-format_whitelist`, in their own process group. Never through a shell.
- Migrations are forward only and must run on PostgreSQL 15. Never edit an applied migration.

## Authentication (Phase 2)

- `auth/` holds the OIDC client (`oidc.rs`, own code on `jsonwebtoken`, DECISIONS 070), cookies and the `returnTo` rule, sealing (`crypto.rs`), the session and bearer rules (`service.rs`), SQL (`repository.rs`) and the extractors (`extract.rs`). `me/` is the caller's own row, `admin/` the admin actions with `audit.rs`.
- Handlers learn who calls only through `Caller`, `WebSession` or `Admin`. `Caller` takes a bearer token when an `Authorization` header is present, else the session cookie, and checks CSRF (`X-R16a-Request: 1` plus `Origin`) on cookie requests that change state.
- `Admin` answers 404 to members and 401 to anonymous callers. Put it first among a handler's extractors and use `ApiPath` for path parameters, which turns a malformed id into 404.
- Sealed values carry labelled AAD (`crypto::session_aad`, `crypto::flow_aad`). Never seal with raw ids.
- Claim refresh takes a lease (`repository::claim_refresh`) and calls Authentik outside any transaction in a spawned task. Never hold a row lock or a pooled connection across a call to Authentik (DECISIONS 075).
- Only `invalid_grant`, a lost access claim and a changed `sub` end a web session. Every other failure is an outage with a one hour grace.
- Setup problems in Authentik (missing scope mappings, no refresh token, missing claim) are `LoginError::Setup`, logged with the request id and shown only as `/signin?error=setup&ref=`.
- Bearer revocation compares `auth_time`, not `iat`, because Authentik keeps `auth_time` through refreshes.
- Users are keyed by the Authentik instance origin and `sub` (DECISIONS 071). Use `AuthState::account_issuer`, never a provider issuer, when looking up users.
- Every new route needs a row in `tests/authz.rs`, which reads `docs/openapi.json`. Regenerate the document first.
- `tests/common/idp.rs` is the mock Authentik. Script it with `add_code`, `set_refresh_mode`, `set_next_refresh_claims`, and sign bearer tokens with `mobile_token`.

## Catalog (Phase 3a)

- `catalog/` holds browse, search and the title page (`handlers.rs`) and admin editing (`admin_handlers.rs`), with `service.rs` for rules and audit and `repository.rs` for SQL. `tmdb/` is the lookup, `client.rs` (HTTP, cache), `draft.rs` (TMDB JSON to `TitleInput`), `reference.rs` (link parsing).
- Every user-facing catalog query carries `status = 'published'`. Admins see drafts only through `/admin/titles`.
- Catalog reads answer through `http::etag::json_with_etag`, lists through `http::cursor`. A new list sort gets its own keyset query, never `OFFSET`.
- Every admin mutation calls `service::audit` in the same transaction.
- Structs that derive `IntoParams` for a query need `#[into_params(parameter_in = Query)]`, `tests/openapi.rs` checks it.
- Routes that need more than 16 KiB set `DefaultBodyLimit` on the route, at most `app::MAX_BODY_LIMIT`.
- `tests/common/tmdb.rs` is the mock TMDB. The authz matrix seeds a published show per row, use `{title}`, `{season}`, `{episode}`, `{user}` in its paths.

## Images and jobs (Phase 3b)

- `images/` holds the TMDB download (`ingest.rs`), presigned URLs (`urls.rs`), the ffmpeg runner (`ffmpeg.rs`) and the worker pipeline (`worker.rs`). `jobs/` is the queue SQL.
- Every new ffmpeg or ffprobe call goes through `images::ffmpeg::run` or an equivalent with the same rules, and gets an argument builder that a unit test pins.
- Worker SQL may only touch the columns the worker role is granted. Test new worker queries as that role, see `tests/images.rs`.
- Job failure classes follow SPEC 7.2. Storage problems requeue with `requeue_storage`, invalid input uses `fail_invalid`, the rest `fail_transient`. `failure_detail` takes fixed texts, never tool output.
- `bench.rs` mirrors the catalog SQL. Change both when a hot query changes, and rerun `seed --bench`.

## Module layout (planned)

`lib.rs`, `app.rs` (router and middleware order), `state.rs`, `openapi.rs`, `bin/api.rs`, `bin/worker.rs`, `config/`, `error/`, `http/` (CSRF, client IP, rate limits, request id, JSON extractor, ETag), `auth/`, `oidc/`, `sessions/`, `catalog/`, `tmdb/`, `uploads/`, `transcode/` (queue, ffmpeg runner, ladder, GC), `images/`, `playback/` (tokens, playlist rewriting, progress), `ratings/`, `favorites/`, `lists/`, `comments/`, `requests/`, `admin/` (audit, moderation), `storage/` (Garage client, presigning, write probe). Tests in `tests/` with `common/` and `fixtures/`.

## How to do common tasks

**Add an endpoint end to end** (same as Zaltys)
1. DTOs in `dto.rs` with `deny_unknown_fields`, `utoipa::ToSchema`, length limits, redacting `Debug` for secrets.
2. Handler in `handlers.rs` with `#[utoipa::path]`, taking `Caller`, `WebSession`, `Bearer` or `Admin`.
3. Rules in `service.rs`, SQL in `repository.rs`.
4. Route in `app.rs`, register in `openapi.rs`, regenerate `docs/openapi.json`.
5. Add the route to the authz matrix in `tests/authz.rs`, or CI fails.
6. Integration tests for invalid input, anonymous, other user and the happy path.

**Add a config value.** Parse in `config/`, error names the variable never the value. Secrets go through the secret parser (no default, placeholder and repetition checks). Document in SPEC 17.

## Running the tests

Unit tests need nothing. Integration tests need the dev stack, and skip with a notice when their variable is missing, unless `R16A_REQUIRE_INTEGRATION=1`, which CI sets, makes a missing service a failure.

```
export SQLX_OFFLINE=true R16A_REQUIRE_INTEGRATION=1 \
  DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
  R16A_TEST_APP_DATABASE_URL=postgres://r16a_stream_app:dev-only-app-password@127.0.0.1:5433/r16a_stream \
  R16A_TEST_WORKER_DATABASE_URL=postgres://r16a_stream_worker:dev-only-worker-password@127.0.0.1:5433/r16a_stream \
  R16A_TEST_S3_ENDPOINT=http://127.0.0.1:3900 R16A_TEST_GARAGE_ADMIN=http://127.0.0.1:3903 \
  R16A_TEST_TRAEFIK_URL=http://localhost:8097
cargo test
```

The auth tests (`auth_web`, `auth_bearer`, `authz`, `log_redaction_auth`) start their own mock Authentik and need only `DATABASE_URL`.

`DATABASE_URL` is the superuser, because `#[sqlx::test]` creates a database per test. The role tests use the real `r16a_stream` database, migrated by the owner, so they see production ownership.

## Gotchas

- `uuidv7()` is PG18 only. Use `gen_random_uuid()`.
- sqlx 0.9 accepts only `&'static str` in `sqlx::query`, a guard against SQL built at runtime. Use the `query!` macros, or literals.
- reqwest is built without a TLS crypto provider (DECISIONS 065). Anything that builds a reqwest client, including tests, must call `r16a_stream_backend::install_crypto_provider()` first, or it panics.
- An unset `R16A_ENV` means production, so a container without it refuses the dev-only credentials. The dev compose services set it.
- The worker reads `R16A_WORKER_DATABASE_URL`, never `R16A_DATABASE_URL` (DECISIONS 066).
- Logging tests use a thread-local subscriber and must live alone in their own test binary, see `tests/log_redaction.rs`.
- `backend/.env` sets `SQLX_OFFLINE=true`, which the query macros also read. Override it with `SQLX_OFFLINE=false` when running `cargo sqlx prepare`.
- `unaccent()` is not immutable. Use `f_unaccent()` in indexes and generated columns.
- `ON DELETE SET NULL (column)` needs PG15, which we have, but test it on `postgres:15`.
- The Garage presign host is the public origin, internal calls use the internal endpoints. Mixing them up gives `SignatureDoesNotMatch`.
- `cargo sqlx prepare` needs the **owner** URL of the migrated `r16a_stream` database. `cargo test` needs the **superuser** URL of `postgres`. Building with `SQLX_OFFLINE=false` and the superuser URL makes every query macro fail with "relation does not exist".
- Tests that hit the login rate limit need a client address. Add `Extension(ConnectInfo(addr))` as a layer, which is what `into_make_service_with_connect_info` inserts. `MockConnectInfo` does not work, the client IP code reads `ConnectInfo` directly.
- Lockfile entries follow the 7-day rule (DECISIONS 072). After `cargo update`, check publication dates and pin back with `cargo update -p name@version --precise older`.
