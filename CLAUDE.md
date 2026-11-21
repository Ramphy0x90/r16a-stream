# r16a-stream

Private, self-hosted streaming of movies and TV shows for members of one Authentik group. Rust (axum) API and transcode worker on PostgreSQL 15 and Garage, an Angular web app and a Flutter mobile app. Working name, the final name and hostname live only in config (`docs/OPERATING.md` section 9).

**Status: Phase 3 done on web and mobile** (2026-11-21), approved pending the owner's local TMDB import test. Phase 4 (upload and transcoding) built 2026-11-21 and awaiting review, open items in `docs/SPEC.md` section 24. Phase 2 gate items and security review fixes done 2026-11-20, Phase 2 approval pending. The command table below was run on the dates it states.

## Phases

The authoritative phase plan is in `docs/SPEC.md` section 24. Read it before starting or scoping a phase. Every user feature ships for web and mobile in the same phase. Admin screens are web only, mobile never gets them.

## Non-negotiable rules

- Agents never commit, push, tag, stash, reset, rebase or switch branches. `git status` and `git diff` are fine. Leave every change in the working tree.
- Verify every library version against its registry (crates.io, npm, pub.dev) before adding or bumping it, never from memory. A dependency not listed in `docs/DECISIONS.md` section B needs the human's approval first.
- A release of a dependency, toolchain, SDK, image or action is adopted only when it is at least 7 days old by its registry publication date. Exception, a release fixing a security vulnerability that affects this project is adopted at once and recorded in `docs/DECISIONS.md` with the advisory and why it applies (DECISIONS 064).
- Never log, return or put in errors any secret: tokens, cookies, session ids, playback tokens, presigned URLs, Garage keys, the TMDB token, DB passwords. DTOs holding such values print `[redacted]` in `Debug`.
- Secrets never have defaults. The api and worker refuse a missing secret and any placeholder such as `CHANGE_ME` at startup.
- No auth bypass, fake login or dev-mode authorisation skip anywhere, not even behind a flag.
- Comments and doc comments explain behaviour, intent, invariants and the non-obvious why, in the native doc format. Never restate the code. No colon and no em dash inside comment sentences.
- When a request conflicts with security, performance or itself, say so and propose a fix. When asking the human to install or decide something, explain why.

## Security invariants

Breaking one of these is a security bug. Details in `docs/SPEC.md`.

- Every query on user data filters on the caller in SQL. Foreign ids and admin routes for non-admins return 404 (SPEC 6.6).
- Admin is decided by the `r16a_stream_admin` claim, refreshed at least every 10 minutes, and enforced on the backend (SPEC 6.3, 6.4).
- The API key cannot write media, the worker key cannot write uploads. The app never holds Garage's admin token, only the read-only `metrics_token` (SPEC 10.1).
- The worker connects as `r16a_stream_worker`, which only has the grants in SPEC 4.9. A new table gets worker grants only by an explicit migration (SPEC 4.9).
- The API never proxies video bytes. Playback tokens and presigned URLs never appear in logs (SPEC 9.5).
- ffmpeg reads only local files, with protocol and format whitelists, argument arrays, never a shell (SPEC 7.3).
- The current or a running transcode attempt is never deleted by GC (SPEC 7.6).
- A source is deleted only after the attempt made from it is ready and verified object by object (SPEC 7.9). Uploads are admitted only if their estimate fits in Garage (SPEC 7.10).
- `Cookie` and `Authorization` never reach Garage (SPEC 10.3).
- Strict CSP without `unsafe-inline` or `unsafe-eval` (SPEC 11.3).

## Architecture

Same origin on one host. Traefik routes `/api` to the backend, `/` to nginx serving the SPA, and the two bucket paths to Garage with method filtering (SPEC 2.2, 10.3). Web signs in through the backend as a BFF with an HttpOnly `__Host-` cookie plus CSRF header and Origin check. Mobile uses Authentik PKCE and bearer access tokens. The worker claims jobs from Postgres, runs ffmpeg in a sandboxed container, and writes HLS output to Garage.

## Repo map

- `backend/` Rust crate, binaries `api` and `worker`, migrations, `.sqlx/` offline cache.
- `web/` Angular app, nginx config, Dockerfile.
- `mobile/` Flutter app.
- `packages/design-tokens/` HSL token source and generated CSS.
- `shared/scripts/` token generator, contrast check, comment style check, other repo tooling.
- `deploy/compose/` dev and prod-like Docker Compose, Garage init, seed media. `deploy/k8s/` production manifests.
- `docs/` SPEC (source of truth), DECISIONS, OPERATING, THREAT_MODEL, generated `openapi.json`.
- Root `package.json` is an npm workspace over `packages/*` and `web`, one lockfile.

## Commands

All verified on 2026-11-18, mobile on 2026-11-20. Node 26.13.1 (`nvm use` reads `.nvmrc`), Rust 1.100.0 from `backend/rust-toolchain.toml`, Flutter 3.50.0 through FVM 4.3.2 from `mobile/.fvmrc` (never the shared SDK at `~/development/flutter`). Install JavaScript dependencies once with `npm ci` at the repository root.

| Task | Command |
|---|---|
| Dev infrastructure (Postgres 15 on 5433, Garage v1.0.1 on 3900 and 3903, scoped init) | `docker compose -f deploy/compose/docker-compose.dev.yml up -d` |
| Dev containers from the real Dockerfiles | add `--profile backend`, `--profile worker`, `--profile web` (nginx on 127.0.0.1:8080) |
| Prod-like stack | `cd deploy/compose && cp .env.example .env` (fill it), then `docker compose up -d --build --wait` |
| Traefik 3.6.7 test harness | `docker compose -f deploy/compose/test/traefik/docker-compose.yml up -d` (needs the dev stack) |
| API, manual mode | `cp backend/.env.example backend/.env` once, then `cd backend && cargo run --bin api` |
| Worker, manual mode | `cd backend && cargo run --bin worker` |
| Migrations | `cd backend && cargo run --bin api -- migrate` with `R16A_MIGRATE_DATABASE_URL`, or automatic on `cargo run` in development |
| Backend format and lint | `cd backend && cargo fmt --check && cargo clippy --all-targets -- -D warnings` |
| Backend tests | `cd backend && cargo test`, integration variables in `backend/CLAUDE.md`. The auth tests run against an in-process mock Authentik and need only the dev Postgres |
| sqlx cache | `cd backend && SQLX_OFFLINE=false DATABASE_URL=<owner url> cargo sqlx prepare -- --all-targets` |
| OpenAPI document | `cd backend && R16A_WRITE_OPENAPI=1 cargo test --test openapi`, then `cd web && npm run gen:api` |
| Supply chain | `cd backend && cargo deny check`, `npm audit --audit-level=high` |
| Web dev server | `cd web && npm start` (http://localhost:4200, proxies `/api` and the bucket paths) |
| Web lint, format, tests | `cd web && npm run lint && npm run format:check && npm test` |
| Web build and checks | `cd web && npm run build && npm run check:bundle && npm run check:i18n` |
| CSP in Chromium | `cd web && npx playwright test e2e/csp.spec.ts` against nginx (`R16A_E2E_BASE_URL`, default http://127.0.0.1:8080) |
| Tokens, contrast, comment rule, tooling tests | `npm run tokens:check`, `npm run contrast:check`, `npm run check:comments`, `npm run test:scripts` |
| Invalid UTF-8 probe | `node shared/scripts/utf8-probe.mjs <base url> http://127.0.0.1:3903/health` |
| Mobile | `cd mobile && fvm install && fvm flutter pub get && fvm flutter analyze && fvm flutter test`, `fvm dart format lib test` |
| Mobile API models | `shared/scripts/gen-mobile-api.sh` after regenerating `docs/openapi.json` |
| Hot query bench | `cd backend && cargo run --bin api -- seed --bench`, development only, report into `docs/perf/queries.md` |
| k8s validation | kubeconform, pinned in Phase 9 with the manifests |

## Conventions

- Feature-oriented folders. Cross-cutting code in `core/`, presentational pieces in `shared/` (web) or `core/ui/` (mobile).
- Backend modules split `dto.rs`, `handlers.rs` (HTTP only), `service.rs` (rules), `repository.rs` (SQL only, `sqlx::query!`).
- Web is standalone, signals, zoneless, OnPush. Mobile is Riverpod with go_router.
- Colours only from semantic HSL tokens. Content language and UI language are different things everywhere.

## Where to look next

[backend/CLAUDE.md](backend/CLAUDE.md), [web/CLAUDE.md](web/CLAUDE.md), [mobile/CLAUDE.md](mobile/CLAUDE.md), [deploy/CLAUDE.md](deploy/CLAUDE.md), [docs/SPEC.md](docs/SPEC.md), [docs/DECISIONS.md](docs/DECISIONS.md), [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md), [docs/OPERATING.md](docs/OPERATING.md).

## Keeping these files current

When a change alters a rule, command, invariant or folder structure documented in a `CLAUDE.md`, update that file in the same change. A design decision made or reversed gets an entry in `docs/DECISIONS.md`. API, data model and flow detail belongs in `docs/SPEC.md`.
