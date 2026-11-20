# r16a-stream

A private, self-hosted streaming platform for movies and TV shows, for the members of one Authentik group. Rust API and transcode worker on PostgreSQL 15 and Garage (S3), an Angular web app and a Flutter mobile app. "r16a-stream" is a working name.

Status: **Phase 2 (auth)**. The stack starts end to end in both local modes, with health checks, security headers, the strict CSP, the design tokens, CI, and sign-in through Authentik on web (BFF with an HttpOnly session cookie) and mobile (PKCE in the system browser). Catalog, upload, playback and the user features arrive in Phases 3 to 8. The plan and every decision are in [`docs/`](docs/).

## What you need locally, and why

| Tool | Version | Why |
|---|---|---|
| Docker with Compose v2 | recent | runs PostgreSQL 15 and Garage v1.0.1, and the prod-like stack |
| Rust via rustup | 1.100.0, pinned in `backend/rust-toolchain.toml` | builds the API and the worker. rustup installs the pinned version on first use |
| Node via nvm | 26.13.1, pinned in `.nvmrc` | Angular CLI, the design-token and contrast scripts, Playwright |
| ffmpeg and ffprobe | 7.1 (Debian trixie) | the worker runs them natively in manual mode |
| sqlx-cli | 0.9.0, same as sqlx | migrations and refreshing the `backend/.sqlx` query cache |
| FVM | 4.3.2 | installs and runs the Flutter version pinned in `mobile/.fvmrc` (3.50.0), see below |

`sqlx-cli` installs with `cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls`.

### Why Flutter comes from FVM

This machine has one shared Flutter SDK in `~/development/flutter`, which another project (Zaltys) depends on at its own version. Upgrading it for r16a-stream would change that project's toolchain under its feet, and staying on its version would hold this app back. FVM (Flutter Version Management) keeps several SDKs side by side in `~/fvm/versions` and picks the one a project pins, so each repository gets exactly the Flutter it declares and nothing global changes. The version lives in one file, `mobile/.fvmrc`, which CI reads too, so a developer machine and CI cannot drift apart. Decision and alternatives are in `docs/DECISIONS.md` entry 068.

Install FVM once from its GitHub release, checking the published digest (the same release and digest CI uses):

```
curl -fsSLO https://github.com/btwld/fvm/releases/download/4.3.2/fvm-4.3.2-linux-x64.tar.gz
echo "17dfced05f5b0cb32398dadd0691acc2bc458e8c4b0045a781cd5aa6c85f6ecb  fvm-4.3.2-linux-x64.tar.gz" | sha256sum -c -
mkdir -p ~/.local/share/fvm-4.3.2 && tar xzf fvm-4.3.2-linux-x64.tar.gz -C ~/.local/share/fvm-4.3.2 --strip-components=1
ln -sfn ~/.local/share/fvm-4.3.2/fvm ~/.local/bin/fvm
cd mobile && fvm install
```

From then on, run every Flutter and Dart command for this repository through FVM, `fvm flutter ...` and `fvm dart ...`. A plain `flutter` still runs the shared SDK. In Android Studio or IntelliJ, set the Flutter SDK path of this project to `mobile/.fvm/flutter_sdk`.

## Run it locally, manual mode

Infrastructure in Docker, the apps natively. This is the daily workflow.

1. Start Postgres 15 (on port 5433) and Garage v1.0.1 (S3 on 3900, admin on 3903). A one-shot init creates both buckets and both scoped keys and proves the API key cannot write media and the worker key cannot write uploads.
   ```
   docker compose -f deploy/compose/docker-compose.dev.yml up -d
   ```
2. Configure the backend. The example holds only published dev-only values, which the backend refuses outside development. Signing in needs the `r16a-stream-dev` and `r16a-stream-mobile-dev` providers on your Authentik (`docs/OPERATING.md` section 3.1). Put their issuer URLs and the dev client secret into `backend/.env`. The API refuses to start without them, because a login that silently does not work is worse than a clear error.
   ```
   cp backend/.env.example backend/.env
   ```
3. Start the API (migrates itself in development) and the worker, in two terminals.
   ```
   cd backend && cargo run --bin api
   cd backend && cargo run --bin worker
   ```
4. Install the JavaScript workspace once at the repository root, then start the web app.
   ```
   nvm use && npm ci
   cd web && npm start
   ```
   Open http://localhost:4200. `ng serve` proxies `/api` to the API and the two bucket paths to Garage, so everything shares one origin, as in production.
5. Mobile, optional.
   ```
   cd mobile && fvm flutter run --dart-define=R16A_API_BASE_URL=http://10.0.2.2:8081 --dart-define=R16A_AUTHENTIK_URL=https://<your authentik>
   ```

Profiles add containers from the real Dockerfiles when wanted: `--profile backend`, `--profile worker`, `--profile web` (nginx with the production CSP on http://127.0.0.1:8080).

## Run it locally, prod-like mode

Every service from the real Dockerfiles, with the production hardening, served on http://localhost:8080.

```
cd deploy/compose
cp .env.example .env      # replace every CHANGE_ME, the commands are next to each value
docker compose up -d --build --wait
```

The worker sits on internal networks only, so ffmpeg has no internet access. Stop with `docker compose down` (add `-v` to delete the data).

## Checks

| What | Command |
|---|---|
| Backend format, lint, tests | `cd backend && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` (integration variables in `backend/CLAUDE.md`) |
| Supply chain | `cd backend && cargo deny check`, `npm audit` |
| Web | `cd web && npm run lint && npm test && npm run build && npm run check:bundle && npm run check:i18n` |
| CSP in a real browser | `cd web && npx playwright test e2e/csp.spec.ts` against the nginx container |
| Tokens and contrast | `npm run tokens:check && npm run contrast:check` |
| Comment rule | `npm run check:comments` |
| Mobile | `cd mobile && fvm flutter analyze && fvm flutter test` |

## Repository map

`backend/` Rust API and worker, `web/` Angular app and nginx, `mobile/` Flutter app, `packages/design-tokens/` colour tokens, `shared/scripts/` repository tooling, `deploy/` compose and Kubernetes, `docs/` specification, decisions, threat model and runbook. Rules for working in each folder are in its `CLAUDE.md`.
