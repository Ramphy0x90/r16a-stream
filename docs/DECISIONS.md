# Decisions

Every significant decision for r16a-stream, with context, options and why. `docs/SPEC.md` has the detail, and section numbers like SPEC 7.6 point there. Add a new entry for every new decision and mark replaced entries as superseded instead of deleting them.

Status values. **brief** means the brief decided it. **proposed** means it still needs approval. **accepted** means approved.

**Phase 0 approval, 2026-11-18.** Every entry 001 to 046 is accepted, with the amendments written into entries 015, 020, 021, 023, 032, 036, 038 and 042 and the new entries 047 to 058. A second round the same day approved all dependencies in section B (060) and settled C14 to C17 (entries 049, 051, 053, 059).

Section A holds design decisions. Section B holds every dependency with the version verified against its registry on 2026-11-16. Section C lists what to re-check after upgrades.

---

## A. Design decisions

### 001 Spec first, Zaltys conventions
2026-11-16 · brief
Context: The project is built by agents in phases and reviewed by a human who wants to learn every step.
Decision: Phase 0 writes only docs. Every phase stops for approval. Conventions, rules and file shapes follow Zaltys unless the brief or this file says otherwise.
Consequences: No code before the spec is approved. Deviations from Zaltys are recorded here.

### 002 Repository layout by part of the system
2026-11-16 · brief
Context: Zaltys uses top-level `backend/`, `web/`, `mobile/`, `packages/`, `shared/`, `deploy/`, `docs/`.
Decision: Same shape, no `apps/` or `services/` buckets, feature-oriented folders inside each app.
Consequences: The leftovers in the working tree from an earlier attempt (`.github/workflows/deploy.yaml` pointing at `services/` and `apps/`, the root `.env.example`, `.gitignore` entries for `/services` and `/apps`) do not match and are replaced in Phase 1.

### 003 One backend crate, two binaries
2026-11-16 · brief
Context: The API and the worker share config, storage, error types and the database schema, but the worker must not carry HTTP routes and the API must not carry ffmpeg.
Options: a Cargo workspace with three crates, or one crate with two binaries.
Decision: One crate `r16a-stream-backend`, `bin/api.rs` and `bin/worker.rs`. The worker binary only wires up the modules it needs.
Consequences: One `Cargo.lock`, one `.sqlx/` cache, one Dockerfile with two targets. The compiler drops unused code per binary, so the worker image does not grow with the API.

### 004 PostgreSQL 15 only, built-in UUIDs
2026-11-16 · brief
Context: The shared instance runs 15. Zaltys was bitten by `uuidv7()` (PG18) in dev.
Decision: Local dev, CI and production all use `postgres:15`. Primary keys use `gen_random_uuid()`. Nothing newer than PG15 is used. The DDL in SPEC 4 was applied to `postgres:15.20` as a non-superuser owner on 2026-11-16 and works, including the trusted extensions.
Consequences: Dev catches version mistakes, unlike Zaltys where dev ran 18.

### 005 Job queue in PostgreSQL with SKIP LOCKED
2026-11-16 · brief
Context: Transcodes, image work and GC need a durable queue with retries.
Options: a broker (NATS, Redis), or Postgres.
Decision: `jobs` table, claim with `for update skip locked`, heartbeats, reaper, `LISTEN/NOTIFY` wake-ups with polling fallback (SPEC 7.2).
Consequences: No extra service to run or secure. Enqueue is transactional with the state change that causes it. Throughput is far beyond what transcoding needs.

### 006 Each language is a full encoded copy
2026-11-16 · brief
Context: Dubbed releases often differ in cut and timing from the original.
Decision: `media_variant` per language with its own video and audio, `rendition` per quality (SPEC 3, 4.5).
Consequences: Storage grows with each language. Switching language swaps the variant and seeks to the same timestamp, which is approximate when cuts differ.

### 007 HLS VOD with fMP4 segments, H.264 and AAC ladder
2026-11-16 · brief
Context: Must play in hls.js, Safari native, AVPlayer and ExoPlayer.
Decision: CMAF fMP4 segments of 6 s, 2 s closed GOPs forced at identical times across rungs, 1080p to 360p capped at the source, x264 CRF 21 with a VBV cap per rung, one shared AAC stereo audio rendition (SPEC 7.4).
Consequences: Universal playback. The VBV cap trades a little quality on hard scenes for predictable bandwidth, so `BANDWIDTH` in the master is honest. Surround and HEVC are later options.

### 008 One ffmpeg pass per ladder, master playlist built by the API
2026-11-16 · proposed
Context: Encoding each rung separately decodes the source four times. The master playlist needs per-session token propagation anyway.
Decision: One ffmpeg process with `split` and one output per rung. The worker stores media playlists. The API generates the master from `renditions` rows.
Consequences: About four times less decode work. The master stays consistent with the database. A crash loses the whole attempt, which retries from the start.

### 009 Encoder backends, x264 default, Quick Sync through the device plugin
2026-11-16 · brief
Context: No dedicated GPU. The ThinkStation's UHD 770 has Quick Sync. It is not a cluster node yet.
Decision: `R16A_WORKER_ENCODER` selects `x264`, `qsv` or `vaapi`. Hardware access only through `gpu.intel.com/i915` from the Intel GPU device plugin. Fallback to x264 when no device or the test encode fails (SPEC 7.5).
Consequences: No privileged pods. Quality and speed numbers are measured in Phase 4, not claimed now.

### 010 Attempt folders, atomic switch, delayed GC
2026-11-16 · brief
Context: Retries and crashes must never leave a variant pointing at half-written output.
Decision: Each attempt writes under `v/<variant>/<attempt>/`. One transaction switches `current_attempt_id`. `encode_gc` every 6 hours, deletes orphans only after 24 hours, never the current or a running attempt (SPEC 7.6).
Consequences: Crash-safe. Storage temporarily holds up to two copies of a re-transcoded variant.

### 011 GC grace covers the longest playback token
2026-11-16 · proposed
Context: Tokens bind to an attempt so a running session keeps working through a re-transcode. GC could otherwise delete an attempt under a session.
Decision: Token lifetime capped by `R16A_PLAYBACK_MAX_LIFETIME_HOURS` (23). Config refuses `R16A_GC_GRACE_HOURS` not greater than it.
Consequences: Titles longer than about 21 hours get a token shorter than runtime plus 2 hours. The player re-requests playback on 401, so this is invisible in practice.

### 012 Garage replication factor 2, writes are retryable, readiness split
2026-11-16 · brief, with C2 in SPEC 21
Context: With two nodes and replication 2, a single node outage blocks all writes but not reads.
Decision: Write failures requeue jobs with exponential backoff and never count as failures. Write probes every 30 s. The worker reports write health in `/readyz`. The API reports only the database, returns `503 storage_degraded` on writing routes, and shows the state to admins (SPEC 7.8).
Consequences: Playback survives a node outage. Uploads resume once both nodes are back. Replication is not a backup (OPERATING 7).

### 013 Scoped Garage keys, ingest staged in uploads
2026-11-16 · brief, with C9 and C10 in SPEC 21
Context: A compromised API must not be able to overwrite media.
Decision: API key reads media and reads and writes uploads. Worker key reads uploads and reads and writes media. TMDB images and subtitles go through uploads and are promoted by worker jobs. The dev init script proves both negative cases.
Consequences: Small extra latency for images and subtitles. The worker becomes the only writer of anything served to users.

### 014 Bucket paths on the app origin through an IngressRoute
2026-11-16 · brief
Context: Same origin avoids CORS, lets the CSP stay `'self'`, and keeps one TLS certificate.
Decision: Router rules match method and query (`GET`/`HEAD` on media, `PUT` with `partNumber` and `uploadId` and no `X-Amz-Copy-Source` on uploads), a priority 1 catch-all answers 403 through an `ipAllowList` of `127.0.0.1/32`. Bucket responses get `Content-Security-Policy: default-src 'none'; sandbox` and `nosniff` (SPEC 10.3).
Consequences: Garage is reachable from the internet only for presigned object reads and part uploads. An object opened in a tab cannot script the app origin.

### 015 Session cookie keeps `__Host-`, cookies stripped before Garage
2026-11-16 · proposed (SPEC 21 C1)
Context: The brief wants the cookie scoped to `/api`. `__Host-` requires `Path=/`. Other apps live on sibling `*.r16a.cloud` hosts.
Options: `__Host-` with `Path=/`, or `__Secure-` with `Path=/api`.
Decision: `__Host-r16a_stream_session`, `Path=/`, `SameSite=Strict`, and a Traefik middleware plus the dev proxy remove `Cookie` and `Authorization` on bucket routes.
Consequences: A sibling host cannot plant or shadow the session cookie. Garage never sees it. Browsers do send it to `/` (the static web app), where nginx ignores it.

### 016 Playback token in the URL path, HMAC with key ids
2026-11-16 · brief, format proposed
Context: Native players cannot reliably send cookies or headers.
Decision: `v1.<kid>.<payload>.<mac>`, payload binds user, variant, attempt and expiry, HMAC-SHA256 with a domain-separation prefix, current and previous key for rotation (SPEC 9.2).
Consequences: Relative URIs in the master carry the token. Leakage is contained by short lifetimes, log redaction and `no-referrer`.

### 017 Hour-rounded presigning for media, day-rounded for images
2026-11-16 · brief for media, proposed for images
Context: Identical URLs are cacheable. SigV4 presigned URLs expire after at most 7 days.
Decision: Media: signing time floored to the hour, expiry token lifetime plus 1 hour. Images: floored to the UTC day, expiry 48 hours (SPEC 8.3, 9.3).
Consequences: Browser and app caches work. A leaked image URL is valid for up to two days, acceptable for posters.

### 018 Image variants made with ffmpeg in the worker
2026-11-16 · proposed
Context: Posters and backdrops need AVIF and WebP at several widths. Decoding untrusted images is attack surface.
Options: the `image` crate plus an AVIF encoder (`ravif`) in Rust, or ffmpeg.
Decision: ffmpeg (`libaom-av1 -still-picture 1`, `libwebp`) inside the hardened worker.
Consequences: No new Rust dependencies. All untrusted media decoding happens in one sandbox. Debian's ffmpeg build must include libaom and libwebp, checked in Phase 3.

### 019 Subtitles always side-loaded as WebVTT
2026-11-16 · brief for mobile, proposed for web
Context: `video_player` takes subtitles as a separate WebVTT file. Putting them in the master as well would mean two code paths.
Decision: The playback start response lists subtitle tracks with presigned URLs. Web uses `<track>`, mobile uses `WebVTTCaptionFile`.
Consequences: One model everywhere. Native AVPlayer subtitle styling is not used on iOS.

### 020 Web BFF with `openidconnect`, tokens sealed at rest
2026-11-16 · brief for BFF, proposed for library and sealing
Context: Tokens stay server side. The refresh token is needed to re-read the admin claim and the ID token for RP-initiated logout.
Options: hand-written code flow on `jsonwebtoken`, or `openidconnect` (as in Zaltys).
Decision: `openidconnect` 4.1.0 for discovery, code flow with PKCE and ID token verification, with our own adapter so it uses our `reqwest` instead of pulling `reqwest` 0.12. Refresh and ID tokens are sealed with XChaCha20-Poly1305 (`chacha20poly1305`) under `R16A_SESSION_KEY`, AAD `r16a-stream:session:v1:<session id>:<field>`.
Amended 2026-11-18: only the refresh token is stored. The ID token was kept as `id_token_hint` for logout, which Authentik 2026.2.1 ignores (049).
**Library part superseded 2026-11-20 by 070**, the sealing part stands.
Consequences: A database leak alone does not yield usable Authentik tokens. `openidconnect` brings `rsa` 0.9 (RUSTSEC-2023-0071, Marvin, affects decryption and signing only, we only verify), ignored in `deny.toml` with that reason like Zaltys. It also brings `chrono` and an older `rand`, accepted for the audited flow code.

### 021 Mobile uses a separate public provider, JWT access tokens validated with `jsonwebtoken`
2026-11-16 · brief, with C6
Context: The mobile app cannot hold a secret. The API validates access tokens offline against JWKS.
Decision: Authentik providers `r16a-stream-web` (confidential) and `r16a-stream-mobile` (public). The API checks mobile tokens with `jsonwebtoken` 11.1.0, RS256 only, `iss`, `aud`, `exp`, `nbf`, 60 s leeway, JWKS cache with rate-limited refetch on unknown `kid` (SPEC 6.4).
Amended 2026-11-18: four providers, prod and dev for each client type (050). The API also requires the `scope` claim to contain `r16a-stream`, because Authentik ID tokens and access tokens share `aud` and claims, and only access tokens carry `scope` (049).
Amended 2026-11-20: `jsonwebtoken` 12.0.0, reviewed in Phase 2 as planned. It reached 7 days on 2026-11-20 and uses the patched `rsa` 0.10, while 11.1.0 uses the affected 0.9 (070). The same verifier now also checks web ID tokens.
Consequences: Four applications to configure. `jsonwebtoken` 12.0.0 was published three days before Phase 0, so 11.1.0 is chosen and 12 reviewed in Phase 2.

### 022 Two boolean claims from one scope mapping
2026-11-16 · proposed (SPEC 21 C3)
Context: The brief wants admin as one boolean, and the API to re-check group membership.
Decision: The mapping returns `r16a_stream_user` and `r16a_stream_admin`, both booleans.
Amended 2026-11-20: two mappings with the same scope name, production and dev, so the CI groups count only on the dev providers (075). Both also return `name` and `preferred_username`, which replace the `profile` scope (074).
Consequences: The API can enforce membership without a groups claim.

### 023 Mobile redirect with a custom scheme and PKCE
2026-11-16 · proposed
Context: Zaltys uses verified https callbacks, which need `assetlinks.json` and `apple-app-site-association` on the host and iOS 17.4 or later.
Decision: `cloud.r16a.stream:/oauth2redirect` (release) and `cloud.r16a.stream.dev:/oauth2redirect` (debug) with PKCE S256, as RFC 8252 allows for native apps. iOS uses `ASWebAuthenticationSession`, which scopes the callback to the session.
Consequences: Simpler, and nothing extra to serve on a hostname that will change. On Android another app could claim the scheme and receive the code, but cannot redeem it without the PKCE verifier.

### 024 Users keyed by issuer and subject
2026-11-16 · brief, refined by 071 (the issuer part is the Authentik instance origin)
Decision: `unique (oidc_issuer, oidc_subject)`, no email requested.
Consequences: Renaming a user in Authentik or changing their email never splits or merges accounts.

### 025 Foreign ids and admin routes for non-admins return 404
2026-11-16 · brief, extended to admin routes
Consequences: No existence oracle. The authz harness asserts the exact status per caller.

### 026 Keyset pagination with opaque cursors, filters in the URL
2026-11-16 · brief
Decision: `{ items, nextCursor }`, cursor is base64url of the last sort key and id, never an offset.
Consequences: Stable infinite scroll under concurrent inserts, constant cost per page.

### 027 Search with the `simple` configuration, an immutable unaccent wrapper and trigrams
2026-11-16 · proposed
Context: Titles are in many languages, so a language-specific stemmer would mangle half of them.
Decision: `search_doc` generated column with `to_tsvector('simple', f_unaccent(...))`, plus a trigram index on the unaccented lowercase name for typos and prefixes (SPEC 4.1, 4.4).
Consequences: Accent-insensitive, typo-tolerant. No stemming, so "running" does not match "run", acceptable for titles.

### 028 Denormalised `title_languages`
2026-11-16 · proposed
Context: The language filter would otherwise join titles, episodes and variants on every page.
Decision: A table maintained in the same transaction that changes a variant's readiness.
Consequences: One index probe per filtered page. One more invariant, covered by tests.

### 029 Append-only audit log
2026-11-16 · proposed
Decision: The app role has `INSERT` and `SELECT` on `audit_log`, `UPDATE` and `DELETE` revoked.
Consequences: A compromised API can add noise but cannot erase history.

### 030 In-memory rate limiting per replica
2026-11-16 · brief
Decision: `governor` GCRA keyed by user or client IP (SPEC 16). Limits are per replica and documented as such.
Consequences: No shared state. With more replicas the limits loosen proportionally.

### 031 Worker NetworkPolicy is the only one in the cluster
2026-11-16 · brief
Context: The rest of the cluster has none. The worker runs ffmpeg on hostile input.
Decision: Worker egress only to Postgres pods, the Garage node IPs and kube-dns. No ingress (SPEC 19.4). The backend has no egress policy because Calico OSS cannot filter by domain name.
Consequences: A compromised worker cannot reach the internet directly, but DNS remains an exfiltration channel (THREAT_MODEL T-WRK-5).

### 032 Scratch on a local PersistentVolume, not hostPath
2026-11-16 · proposed (SPEC 21 C7)
Decision: Static local PV, `WaitForFirstConsumer` (SPEC 19.3). Amended 2026-11-18: node chosen by the label `r16a.cloud/transcode=true`, never a hostname, default 200 Gi, concurrency 1 (056).
Consequences: Keeps Pod Security `restricted`. The worker is pinned to the labelled node, as it would be with hostPath.

### 033 Toolchain versions
2026-11-16 · accepted
Amended 2026-11-18: Rust 1.100.0 and Node 26.13.1 were installed next to the existing toolchains (rustup and nvm, defaults unchanged). Flutter stays at the shared 3.44.6 SDK for now (SPEC Q13), which every chosen package supports.
Amended 2026-11-20: Flutter 3.50.0 through FVM, pinned in `mobile/.fvmrc` (068).
Decision: Rust 1.100.0 (stable, pinned in `rust-toolchain.toml`), Node 26.13.1 (Active LTS since 2026-10-28, Angular 22 supports `>=26`), Angular 22.2, TypeScript 6.0.3 (Angular 22.2 requires `>=6.0 <6.1`, so TypeScript 7 is not usable yet), Flutter 3.50.0 with Dart 3.14.
Consequences: Node differs from Zaltys (24.22.2). Both are LTS, nvm switches per repo through `.nvmrc`.

### 034 Own ICU-subset transpiler for Transloco
2026-11-16 · brief
Context: `@jsverse/transloco-messageformat` compiles messages with `new Function`, which needs `'unsafe-eval'`.
Decision: A small parser for interpolation, `plural` and `select` that builds an AST once per message and evaluates it with `Intl.PluralRules` (SPEC 13).
Consequences: Strict CSP holds. Unsupported ICU syntax (`selectordinal`, number skeletons) fails at load with a clear error.

### 035 Design tokens as one HSL source, generators in `shared/scripts`
2026-11-16 · brief
Decision: `packages/design-tokens/tokens.json` is the source. `shared/scripts/tokens-build.ts` emits CSS and Dart, `contrast-check.ts` enforces WCAG 2.2 AA. Both run with Node's built-in TypeScript type stripping, no dependencies.
Consequences: Web and mobile cannot drift. CI fails on drift and on contrast.

### 036 Garage pinned at v1.0.1, compatibility test suite
2026-11-16 · brief, with C4
Context: The cluster runs 1.0.1. Newer 1.x (1.3.1) and 2.x (2.5.0) exist.
Decision: Dev, CI and tests use `dxflrs/garage:v1.0.1`. `tests/garage_compat.rs` exercises every S3 call we use. Confirmed on the cluster 2026-11-18 (`garage --version` on garage-00). Entry 055 compares 1.3.1 and 2.x for the upgrade decision, section C lists what to re-check.
Consequences: We cannot use features added after 1.0.1. Upgrading Garage is an operations task with a test run first.

### 037 Traefik websecure read timeout 300 s
2026-11-16 · brief
Decision: Cluster-wide `respondingTimeouts.readTimeout: 300s` on `websecure` (OPERATING 6).
Consequences: 50 MiB parts survive slow uplinks. Slow-request attacks hold connections five times longer on every app (THREAT_MODEL T-INF-2).

### 038 Garage endpoint failover
2026-11-16 · proposed (SPEC 21 C5)
Decision: Traefik Retry middleware (2 attempts) on the media router only, so it applies to GET and HEAD and never to upload PUTs, whose retries belong to the browser. A `ServersTransport` dial timeout of 2 s on both bucket services.
Amended 2026-11-18 after the Phase 1 harness: retry alone was not enough. With two endpoints the round-robin pointer moves twice per retried request, so every new read started on the dead node and waited 2 s, and parallel reads sometimes got 504. Reads now use their own Traefik service `garage-read` with a passive health check (`maxFailedAttempts: 1`, `failureWindow: 30s`), measured at 2 s for the first read and about 1 ms for every later one. Writes use `garage-write` without it, because Garage answers every write with a 5xx while a node is down and a passive check would then mark the healthy node down too (SPEC 10.3). The API and worker hold an endpoint list and fail over themselves, marking an endpoint down for 30 s after a connection error.
Consequences: A dead node costs one fast retry instead of half the requests failing. A VIP would be cleaner and can replace this later.

### 039 Bundle budget checked on gzip size
2026-11-16 · proposed (SPEC 21 C8)
Decision: `npm run check:bundle` gzips initial chunks and fails above 200 KB. Angular budgets stay on raw size as a second guard.

### 040 Web performance measured with Playwright and CDP throttling
2026-11-16 · proposed
Context: LCP and INP budgets need a repeatable check. Lighthouse CI would be another dependency tree.
Decision: Playwright (already needed for E2E) with CDP CPU throttling and a network profile, reading `PerformanceObserver` entries (SPEC 15).
Consequences: No extra dependency. Numbers are lab numbers, not field data.

### 041 hls.js without its worker
2026-11-16 · proposed
Decision: `enableWorker: false`. Our fMP4 segments need no transmuxing, and a blob worker would need `worker-src blob:`.
Consequences: Slightly more main-thread work on segment append, negligible for fMP4.

### 042 OpenAPI generated by utoipa, clients typed from it
2026-11-16 · brief, tools proposed
Decision: `utoipa` 6.0.0 generates `docs/openapi.json`, CI fails on drift. Web uses `openapi-typescript` (types only). Mobile uses `swagger_parser` (approved 2026-11-18, dev dependency, models only), generated code committed and regenerated by `shared/scripts/gen-mobile-api.sh`, CI fails on drift.

### 043 Mobile image cache keyed on the object key
2026-11-16 · proposed
Context: Image URLs change daily (017), which would defeat a URL-keyed cache.
Decision: `cached_network_image` with `cacheKey` set to the object key and `memCacheWidth` at display size.
Consequences: Images are downloaded once per device until evicted.

### 044 No Angular CDK
2026-11-16 · proposed
Context: CDK virtual scroll handles lists, not responsive grids, and dialogs can use native `<dialog>`.
Decision: A small windowed grid on `IntersectionObserver`, native `<dialog>` with focus handling.
Consequences: One dependency fewer. We own the grid code and its tests.

### 045 Images and subtitles uploaded as single-part multipart
2026-11-16 · proposed
Context: The uploads route only allows `PUT` with `partNumber` and `uploadId`.
Decision: Manual posters use a one-part multipart upload. Subtitles go through the API body (2 MiB), which stages them in uploads.
Consequences: One upload path to secure.

### 046 Favorites cannot be shared
2026-11-16 · proposed
Context: The brief allows sharing a list. Favorites is a system list.
Decision: Only custom lists are shareable.
Consequences: Favorites stay private. A user who wants to share favorites makes a custom list.

### 047 Sources deleted after a verified transcode by default
2026-11-18 · accepted (new requirement)
Context: Garage has about 950 GB usable with replication 2, and every language is a full copy. A 2 h source can be 10 to 60 GB, several times its encodes.
Options: keep sources (re-encode anytime, half the library space gone), delete them (space back, re-encode needs a new upload).
Decision: `R16A_SOURCE_RETENTION=delete` by default, `keep` as an option, optional delay. The API deletes, because only its key writes uploads, and only after the current attempt from that upload is `ready` and `verified_at` is set (SPEC 7.9).
Consequences: Re-transcode is unavailable without a source (`409 source_unavailable`). A later codec change (HEVC, AV1) means re-uploading. Backups must copy encodes instead of sources (OPERATING 7).

### 048 Output verification before the switch
2026-11-18 · accepted
Context: Deleting the source makes the encode the only copy, so "succeeded" must mean "every object is in Garage".
Decision: After upload the worker lists the attempt prefix back, compares every key and size with the local files and parses the playlists. Only then `verified_at` and `total_bytes` are set and the variant switches. The database refuses a `ready` attempt without them.
Consequences: One extra list per attempt. A silently lost write cannot lead to a deleted source.

### 049 Authentik 2026.2.1 behaviour the design relies on
2026-11-18 · accepted, verified in the 2026.2.1 source
Facts: claims from scope mappings are in JWT access tokens when "Include claims in id_token" is on. The token endpoint does not re-evaluate application policies for code or refresh grants. Scope mappings re-run on every refresh with `request.user` set to the token's user. Refresh tokens rotate on every use and reuse fails. The end-session view ignores `id_token_hint` and `post_logout_redirect_uri`.
Decision: The `r16a_stream_user` claim (with `is_active`) is the removal check after login. Web refresh is single-flight per session through a row lock. Mobile refresh is single-flight in the client. No ID token is stored. Access tokens must carry `scope` with `r16a-stream` (SPEC 6.1). Access tokens last 5 minutes, refresh tokens 30 days.
Amended 2026-11-18 (C17): logout returns to the app through a provider-specific invalidation flow, User Logout stage then a static Redirect stage, one flow per provider, default flows untouched (SPEC 6.1.1). Verified in the source that the Redirect stage exists and that stages after User Logout still run. Phase 2 confirms it in the browser and on both phones.
Consequences: Correct behaviour on 2026.2.1. Every one of these facts is in section C to re-check on upgrade.

### 050 Four Authentik providers
2026-11-18 · accepted
Decision: `r16a-stream` (confidential), `r16a-stream-mobile` (public), `r16a-stream-dev` (confidential, localhost callbacks, also used by CI), `r16a-stream-mobile-dev` (public, scheme `cloud.r16a.stream.dev`). Same scope mapping and group binding on all four (SPEC 6.1).
Consequences: No dev or CI secret can mint production sessions, and a debug phone build never holds production tokens.

### 051 CI test users with MFA excluded by attribute
2026-11-18 · accepted
Decision: `r16a-stream-ci-user` and `r16a-stream-ci-admin`, user type external, a negated expression policy matching exactly these two usernames on the MFA stage binding only, failure result pass, binding engine mode changed from `any` to `all` (needed, otherwise the existing policy already lets the stage run), passwords in GitHub secrets, Playwright only against the ephemeral CI stack (SPEC 6.7). Amended 2026-11-18: username match replaces the attribute, as you asked.
Alternative noted: TOTP devices for the CI users with the secret in GitHub, which needs no flow change.
Amended 2026-11-20 after the security review (075): the CI users are in their own groups `r16a-stream-ci-users` and `r16a-stream-ci-admins`, which only the dev mapping counts and only the dev applications bind. Before, they were in `r16a-stream-users` and `r16a-stream-admins`, which the production applications bind and the shared mapping counted, so a leaked CI admin password would have given admin on production with a password alone.
Consequences: The shared authentication flow gets one policy binding. Every other application must exclude these users, because Authentik apps without bindings are open to all (OPERATING 3.2).

### 052 Separate worker database role with column grants
2026-11-18 · accepted
Decision: `r16a_stream_worker`, created by `provision.sql`, `init-roles.sh` and CI, with only the table and column grants in SPEC 4.9, written in migrations. No default privileges for it.
Consequences: A compromised worker cannot read users, sessions, lists, comments, history or audit, and cannot change titles beyond `listed_at`. Each new table needs an explicit decision about worker access. `tests/db_roles.rs` proves the boundary.

### 053 Garage capacity from the read-only metrics token
2026-11-18 · accepted
Context: Admission checks and the admin view need free space. In 1.0.1 per-bucket usage needs the full admin token, which can create keys and grant rights.
Decision: The API reads `garage_local_disk_avail`, `garage_local_disk_total`, `garage_replication_factor` and the node counters from `/metrics` with `metrics_token` and stores them in `garage_capacity`. Usage per title comes from our own byte counts. Logical free = `min(avail) x nodes / replication`. Uploads are admitted only if the estimate fits above a floor, with reservations (SPEC 7.10).
Admin port facts for 1.0.1 (source, tested in Phase 1): `/health` and `/check` need no token, `/metrics` needs `metrics_token` and is public if that token is unset, every other endpoint needs `admin_token` and is refused if that is unset.
Amended 2026-11-18 (C15): while capacity is unknown (port not ready, stale rows, token refused) the page shows "unknown" and admission uses a fixed in-flight budget `R16A_STORAGE_UNKNOWN_BUDGET_BYTES` (default 100 GB) instead of failing.
Consequences: Port 3903 must be reachable from the cluster nodes. The estimate is conservative, so a nearly full Garage refuses uploads a little early.

### 054 Traefik 3.6.7 baseline, notes for 3.7
2026-11-18 · accepted
Context: The cluster runs 3.6.7 and 3.7.13 until aligned.
Decision: Only 3.6.7 features. Verified in the 3.6.7 source: matchers `PathRegexp`, `QueryRegexp`, `HeaderRegexp`, `Method`; per-router `observability.accessLogs` in the IngressRoute CRD; header deletion through an empty `customRequestHeaders` value; `ServersTransport` dial timeout; EndpointSlice discovery for Services.
What differs on 3.7.13:
- Retry gains `timeout`, `status`, `maxRequestBodyBytes`, `disableRetryOnNetworkError` and `retryNonIdempotentMethod`. On 3.7 a retry could be bounded by `timeout`. PUT counts as idempotent there, which is one more reason the uploads router never gets the middleware.
- The CRD no longer limits active `healthCheck` to ExternalName services, so on 3.7 the `garage` service can get an active health check and drop a dead node instead of paying the dial timeout. **This is a one-manifest switch**: in `deploy/k8s/ingress.yaml`, add to the media and uploads services of the IngressRoute
  ```yaml
  healthCheck:
    path: /
    port: 3900
    status: 403
    interval: 5s
    timeout: 2s
  ```
  The check must **not** use Garage's `/health`. In 1.0.1 that endpoint computes health from the write quorum, which is 2 with replication 2, so with one node down the surviving node answers 503 "reads and writes will fail" although reads still work. Traefik would then drop both nodes and playback would stop. An anonymous `GET /` on the S3 port answers 403 on any live node, which is exactly "this node is up". The switch is commented out in the manifest until every Traefik replica runs 3.7.
- The router observability type in the CRD moved to its own struct with the same YAML fields, so manifests do not change.
- Passive health checks work in both. The 3.6.7 CRD field comment mentions ExternalName services, but `buildServersLB` applies them to every service type, and the harness proved they work. They count any 5xx as a failure, so they are used only on the read service (038).
- With the passive check in place, the 3.7 active check mainly adds proactive recovery detection, so one read per 30 s no longer pays the 2 s re-test.

### 055 Garage upgrade comparison (1.0.1 to 1.3.1 or 2.x)
2026-11-18 · information for your decision, from the release notes of 1.1.0 to 2.5.0
**1.3.1** (no breaking changes from 1.0.x, data format compatible, can go back within 1.x)
- Simpler for us: nothing structural. The design is identical.
- Fixes and features that matter: `x-amz-content-sha256` no longer forced into signed headers (1.3.1, helps header-auth clients, not our query-auth calls); access keys used are logged per request (1.3, useful for audits); blocks are no longer written to draining nodes (1.1, matters when a third node is added or one is replaced); limit on simultaneous block reads (1.3, steadier under many players); `block_max_concurrent_writes_per_request` (1.3.1, tuning for 50 MiB parts); several crash and error-code fixes, `CompleteMultipartUpload` returns a Location.
- Risk: low. Recommended as the first step.

**2.x** (current 2.5.0)
- Breaking: admin API v1 mostly replaced by v2, `replication_mode` config removed in favour of `replication_factor` plus `consistency_mode`, CLI reworked, and from 2.5.0 no downgrade once a node is upgraded. 2.5.0 restricts lifecycle endpoints to owner keys.
- Simpler for us: multiple admin tokens scoped to endpoints, so the API could get a token limited to reading bucket usage and cluster health. That would replace the metrics parsing, give exact per-bucket bytes instead of our own counts, and give a direct health endpoint (`GetClusterHealth`, `garage health`). Access keys can expire, which makes key rotation enforceable. `--single-node` and default keys would shorten the dev init a little, though we still need two keys and two buckets.
- Security fixes in the 2.x line that 1.3.1 does not list: a panic on invalid UTF-8 in percent-decoded URLs (2.5.0) and SigV4 canonical header whitespace handling (2.4.0). Whether 1.x is affected by the first is not stated. Our bucket routes are internet-facing, so this is worth checking before deciding.
- Metrics dependencies were swapped in 2.5.0, so the metric names we parse must be re-checked.
- What would change in our code: the capacity source (053) could switch to the v2 admin API, the compose init uses new CLI syntax, and section C runs in full.
- Risk: medium, one-way upgrade. Test with the compat and degraded suites against the target image first.

**Health endpoint caveat (all of 1.x):** `/health` reports the write quorum, so with one of two nodes down the surviving node says "unavailable" while it still serves reads. Nothing in our design uses it for routing (054).

**Invalid UTF-8 test against 1.0.1 (Phase 1, 2026-11-18).** Requests with invalid UTF-8 in the path and the query, percent-encoded and as raw bytes, on the S3 port and the admin port, unsigned and signed with the dev keys. Tools: `backend/tests/garage_compat.rs` (automated, direct) and `shared/scripts/utf8-probe.mjs` (direct and through each proxy).

| Path to Garage | Result |
|---|---|
| direct, S3 port 3900 | 400 `Invalid UTF-8` for invalid keys and query keys, 403 where the signature check runs first. Signed requests with invalid keys also 400 |
| direct, admin port 3903 | 400 for every case |
| through Traefik 3.6.7 (harness) | same answers as direct, the deny router catches non-object paths |
| through nginx (compose, prod-like) | same answers as direct |
| through `ng serve` (dev only) | Garage unaffected. Two cases answered 500 by the dev server's own URL decoder, a dev tool issue, not Garage |

In every run Garage stayed up, logged no panic, restarted zero times, and `/health` stayed 200. **Garage 1.0.1 does not crash on these inputs, so no Traefik mitigation is required.** Limit: signed requests carrying invalid bytes in the *query* failed the signature check before Garage parsed them, so the post-authentication query parsing was not reached. A free hardening that needs no Garage change, recommended but not required: tighten the media and uploads routers to the object key characters our code ever produces, `PathRegexp(`^/r16a-stream-media/[A-Za-z0-9/_.-]+$`)`, so a percent-encoded byte never reaches Garage at all.

### 056 Worker placed by a node label, scratch sized by config
2026-11-18 · accepted
Decision: `nodeSelector` `r16a.cloud/transcode=true`, local PV `nodeAffinity` on the same label, default 200 Gi, `R16A_WORKER_TRANSCODE_CONCURRENCY` default 1, x264 default until the ThinkStation joins (SPEC 19.2, 19.3).
Consequences: Moving the worker means moving the label and recreating the PV on the new node (OPERATING 10).

### 057 Admission control for uploads with reservations
2026-11-18 · accepted
Decision: Before an upload starts, `source + ladder estimate` must fit in logical free space minus other reservations minus a floor. Admission is serialised by an advisory lock. Reservations are released when the transcode ends. Unknown capacity fails closed (SPEC 7.10).
Consequences: Two admins cannot overcommit the same space. A stale metrics endpoint blocks new uploads until it recovers, which the admin page explains.

### 058 Playback route split out for access logs
2026-11-18 · accepted
Decision: An IngressRoute router for `/api/v1/play/` with access logs off, next to the normal Ingress for the rest of `/api`.
Consequences: Tokenised paths never reach Traefik's access log, everything else on `/api` stays logged.

### 059 Admin action to revoke a user's sessions
2026-11-18 · accepted
Context: Refresh tokens last 30 days and Authentik does not re-check the group on refresh. Removal takes effect within 5 minutes on its own, but sometimes it must be immediate.
Decision: `POST /admin/users/{id}/revoke-sessions` deletes the user's web sessions and sets `users.sessions_revoked_at`. Bearer tokens with `iat` at or before that time are refused. Paired with deleting the user's sessions and OAuth tokens in Authentik (SPEC 6.8, OPERATING 8).
Consequences: One extra column read per bearer request, served from the same user lookup that provisioning already does.

### 060 Dependency approval and its constraints
2026-11-18 · accepted
Decision: Every dependency in section B is approved, with these constraints:
- `reqwest` with `default-features = false` and rustls only. No OpenSSL anywhere in the tree. `deny.toml` bans `openssl-sys` and `openssl`, so a transitive pull fails CI.
- `sqlx-cli` is a developer and CI tool only, pinned to the same version as `sqlx` (0.9.0).
- ESLint (angular-eslint), Prettier (configured as in Zaltys) and jsdom (Vitest environment) are devDependencies.
Consequences: Phase 1 adds them as listed. Anything else still needs approval.

### 061 Dev proxy as a module, `proxy.conf.mjs`
2026-11-18 · accepted in Phase 1, deviation from the brief's file name
Context: The brief names `web/proxy.conf.json`. The bucket paths must lose `Cookie` and `Authorization` before Garage, as in Traefik (C1). The JSON format can set headers but not remove them.
Decision: `web/proxy.conf.mjs` with a `configure` hook that removes both headers on the bucket paths. Verified with a header echo in Phase 1.
Consequences: One JavaScript file instead of JSON. The Garage target can be overridden with `R16A_DEV_GARAGE`, which the stripping check uses.

### 062 openapi-typescript runs on TypeScript 6 through an npm override
2026-11-18 · accepted, confirmed 2026-11-20
Context: `openapi-typescript` 7.13.0, the latest, declares a peer dependency on TypeScript 5. Angular 22.2 requires TypeScript 6.0. No release accepts 6 yet.
Options: `--legacy-peer-deps` (mutes every peer check, rejected), a second TypeScript 5 only for the generator, or an npm `overrides` entry that hands it our TypeScript 6.0.3.
Decision: The override, `"overrides": { "openapi-typescript": { "typescript": "$typescript" } }` in the root `package.json`, which therefore lists `typescript` 6.0.3 as its only devDependency. The tool uses the compiler API only to print declarations, its output is type-checked by our own build, and CI regenerates it and fails on drift.
Consequences: If a future TypeScript breaks the generator, CI shows it at once.
Removal: delete the override, and `typescript` from the root devDependencies if nothing else needs it there, as soon as an openapi-typescript release officially accepts TypeScript 6 in its peer range (and is at least 7 days old, 064). Checked at every phase gate and listed in OPERATING 13 as a before-production item.

### 063 Dev Postgres on port 5433, distinct compose project names
2026-11-18 · accepted in Phase 1
Context: Port 5432 on this machine is taken by the Zaltys dev database and by a container left from the earlier r16a-stream attempt (`r16a-stream-postgres-1`, compose project `r16a-stream`).
Decision: The dev compose publishes Postgres on 127.0.0.1:5433 (`R16A_DEV_DB_PORT`). Compose projects are `r16a-stream-dev` (dev), `r16a-stream-local` (prod-like) and `r16a-stream-traefik-test`, so none of them adopts or removes the old project's containers. An empty port variable asks Docker for a free port, which CI uses for parallel runs.
Consequences: The old container is left alone, you decide whether to remove it and its volume.

### 064 Releases are adopted only after 7 days
2026-11-18 · accepted as a standing rule 2026-11-20, amended
Context: Several packages released new versions on the day they were to be added: Angular framework 22.3.0 and 22.2.7, rusty-s3 0.12.0, jsonwebtoken 12.0.0 (three days old), cargo-deny-action v2.1.2.
Decision: Pin the newest version that is at least a few days old and was verified earlier (Angular framework 22.2.6, CLI and build 22.2.7, rusty-s3 0.11.0). Angular packages are pinned exactly, not with a range. Newer versions are reviewed at the next phase or through Dependabot.
Consequences: A few days behind upstream, in exchange for not being the first to install a broken or compromised release.
Standing rule (2026-11-20): a release of any dependency, toolchain, SDK, image or action is adopted only when it is at least 7 days old, counted from its publication date in the registry. The one exception is a release that fixes a security vulnerability affecting r16a-stream, which is adopted at once with a DECISIONS entry naming the advisory and why it affects us. Dependabot pull requests younger than 7 days wait. The rule is in the root `CLAUDE.md`.

### 065 One TLS crypto provider, ring
2026-11-18 · accepted in Phase 1
Context: sqlx uses rustls with ring. reqwest 0.13's `rustls` feature pulls aws-lc-rs, and with two providers in one binary rustls refuses to pick one.
Decision: reqwest with `default-features = false` and `rustls-no-provider`, and `rustls` (already in the tree) as a direct dependency only to install ring at startup in both binaries and in the tests. `deny.toml` bans aws-lc-rs and aws-lc-sys next to OpenSSL.
Consequences: No C crypto library is compiled, and builds stay simple.

### 066 The worker reads its own database variable
2026-11-18 · accepted in Phase 1
Context: In manual mode both binaries read `backend/.env`. A shared `R16A_DATABASE_URL` would silently run the worker as the app role and hide every grant mistake.
Decision: The worker reads `R16A_WORKER_DATABASE_URL`, the API `R16A_DATABASE_URL`.
Consequences: Grant boundaries hold in development too.

### 067 Prod-like Garage secrets come from the environment
2026-11-18 · accepted in Phase 1
Context: Garage 1.0.1 reads `GARAGE_RPC_SECRET`, `GARAGE_ADMIN_TOKEN` and `GARAGE_METRICS_TOKEN` (verified in its source).
Decision: `deploy/compose/garage/garage.prod-like.toml` holds no secrets, compose passes them from `.env`. The dev file keeps its published dev-only values.
Consequences: The same pattern fits the production VMs, where the tokens can come from the service environment instead of the config file.

### 068 Flutter pinned per repository with FVM
2026-11-20 · accepted (answers SPEC Q13)
Context: DECISIONS 033 chose Flutter 3.50.0, but the SDK at `~/development/flutter` (3.44.6) is shared with Zaltys and must not be upgraded for this repo. CI had no mobile job at all.
Options: upgrade the shared SDK (breaks Zaltys' pin), stay on 3.44.6 (keeps us on an SDK the spec already moved past), a second SDK managed by hand (no record of which version the repo expects), or FVM.
Decision: FVM 4.3.2, the standalone binary from the btwld/fvm GitHub release, verified by its SHA-256 digest and installed in `~/.local/share/fvm-4.3.2` with a link in `~/.local/bin`. `mobile/.fvmrc` pins Flutter 3.50.0 (released 2026-11-11, so it passes 064) and is the only place the version lives. SDKs live in `~/fvm/versions`, `mobile/.fvm/` is a per-checkout link and ignored by git. `updateVscodeSettings` and `updateGitIgnore` are off so FVM never edits files we own. Every Flutter and Dart command for this repo runs as `fvm flutter ...` and `fvm dart ...`. CI gets its own workflow, `.github/workflows/mobile.yaml`, which installs the same FVM release by digest and runs `fvm install` from `.fvmrc`.
Consequences: Zaltys keeps `~/development/flutter` untouched. The move to 3.50 bumps the Dart constraint to `^3.14.0` and `intl` to 0.20.3, which 3.50's `flutter_localizations` pins. Dependabot does not track FVM or `.fvmrc`, so both are reviewed by hand at each phase gate. Flutter 3.50.1 appeared on 2026-11-20 and is reviewed after 2026-11-27.

### 069 Material and Cupertino from their own packages
2026-11-20 · accepted
Context: Flutter 3.50 deprecates `package:flutter/material.dart`, `GlobalMaterialLocalizations` and `GlobalCupertinoLocalizations` in favour of the `material_ui` and `cupertino_ui` packages (publisher flutter.dev). `flutter analyze` fails on the deprecation infos.
Decision: `material_ui` 1.11.0 (2026-11-09) and `cupertino_ui` 1.1.6 (2026-11-03), the newest releases older than 7 days (064). Code imports `package:material_ui/material_ui.dart`, never `package:flutter/material.dart`. `flutter_localizations` is still needed for `GlobalWidgetsLocalizations` and is imported with `show` for it alone, because it still exports the deprecated Material and Cupertino delegates under the same names.
Consequences: Two more packages to follow, both published by flutter.dev. A short-lived per-file ignore during the move was removed the same day.

### 070 Own OIDC client on `jsonwebtoken`, `openidconnect` dropped
2026-11-20 · accepted (Phase 2 question, supersedes the library part of 020)
Context: `openidconnect` 4.1.0, the latest, depends on `rsa` 0.9, which carries RUSTSEC-2023-0071 (Marvin, timing leak of the private key). We only verify signatures with public keys, so it does not reach us, but `cargo deny` fails without an ignore. It also pulls `chrono`, `oauth2`, `rand` 0.8, `thiserror` 1 and older `hmac`, `sha2` and `base64` next to ours. `jsonwebtoken` 12.0.0 uses the patched `rsa` 0.10 and the same `hmac` 0.13 and `sha2` 0.11 as we do.
Options: keep `openidconnect` with a `deny.toml` ignore and two verifiers in the tree, or a small own client.
Decision: `backend/src/auth/oidc.rs`, about 400 lines over our reqwest. Discovery with an issuer equality check and no scheme downgrade, the JWKS cache with a refetch at most once a minute (T-AUTH-10), code exchange with PKCE and `client_secret_basic`, refresh, RFC 7009 revocation, no redirects followed, responses capped at 256 KiB. Signatures, `iss`, `aud`, `exp` and `nbf` are checked by `jsonwebtoken` 12.0.0 (`rust_crypto`, RS256 only, `kid` required). Protocol rules (state bound to a flow cookie, nonce, `azp`, `scope`, both access claims, `iat` not in the future) are ours and covered by `tests/auth_web.rs` and `tests/auth_bearer.rs` against an in-process mock Authentik. Its RSA key is generated at test time with `rsa` 0.10 as a dev-dependency (the same crate `jsonwebtoken` already pulls), so no private key is committed and secret scanners stay quiet. `rsa`, `crypto-bigint` and `crypto-primes` build with opt-level 3 in dev, which keeps key generation under a second.
Consequences: One verifier for web ID tokens and mobile access tokens, a clean `cargo deny` without ignores, fewer crates. We own the protocol code, so a change in Authentik's token or discovery shape is ours to notice. Section C lists what to re-check.

### 071 One account per person across the four providers
2026-11-20 · accepted (found in Phase 2)
Context: SPEC 6.5 keyed users by `(iss, sub)`. Each Authentik provider has its own issuer (SPEC 6.1, issuer mode "Each provider has a different issuer"), so the same person signing in on the web and on the phone would have got two separate accounts, with their own favorites, progress and lists.
Verified in the 2026.2.1 source: with subject mode "Based on the User's hashed ID", `sub` is `user.uid`, which is `sha256("<user id>-<install id>")` (`core/models.py`, `providers/oauth2/id_token.py`). It is the same for one user across every provider of one Authentik instance, and different on another instance.
Decision: `users.oidc_issuer` stores the Authentik instance origin (for example `https://auth.r16a.cloud`), not the provider issuer. Tokens are still verified against their exact provider issuer and audience. The API refuses to start when `R16A_OIDC_ISSUER` and `R16A_OIDC_MOBILE_ISSUER` are on different origins, because subjects are only comparable within one instance. The subject mode must stay "hashed user ID" on all four providers (OPERATING 3.1).
Consequences: Web and mobile share favorites and progress. Changing the subject mode on a provider would split accounts, so it is in the checklist. Development and production use different databases, so their users never mix even on one Authentik.

### 072 Lockfiles follow the 7-day rule, Phase 1 entries pinned back
2026-11-20 · accepted (applies 064)
Context: An audit of `backend/Cargo.lock` against crates.io publication dates found 23 crates younger than 7 days, among them `sqlx` 0.9.1 (published 2026-11-18, the day Phase 1 locked it), `uuid` 1.29.3, `bytes` 1.12.2, `jiff` 0.2.46, `zeroize` 1.9.2, `brotli` 9.1.3 and `regex-automata` 0.4.19. None was a fix for a vulnerability affecting us.
Decision: each was pinned with `cargo update --precise` to its newest compatible release that is at least 7 days old, for example `sqlx` 0.9.0 (which also matches `sqlx-cli` 0.9.0 again), `uuid` 1.29.2, `jiff` 0.2.43. `rand` is 0.10.3, not 0.10.4 (published 2026-11-15). The audit script ran from the session scratchpad. A repeatable `shared/scripts` version is proposed for Phase 3.
Consequences: Dependabot or `cargo update` would move these forward again, so lockfile updates are reviewed against the rule.
Amended 2026-11-20, npm: 36 of 550 lockfile packages were younger than 7 days, all development and build tooling, none in the browser bundle. Direct pins moved to `eslint` 10.14.0, `typescript-eslint` 8.72.2, `angular-eslint` 22.6.0 and `@playwright/test` 1.65.0 (Chromium build 1254, CI image `v1.65.0-noble` by digest). The lockfile was then resolved afresh with `npm install --before=<now minus 7 days>`, which treats newer releases as nonexistent. Result 0 of 548 young, Angular, TypeScript and the other runtime packages unchanged, 37 tooling versions moved. Conflict noted and resolved by the rule: `typescript-eslint` 8.72.2 depends on a `minimatch` with GHSA-g3hv-75wg-m249 (moderate, denial of service through crafted glob patterns), which the younger 8.73.0 fixes. It does not affect us, because the globs come only from our own lint config on developer machines and CI, so the 7-day rule applies and `npm audit` shows 15 moderate findings from that one advisory until 8.73.0 can be adopted on 2026-11-23. CI fails only on high.

### 073 Android build toolchain for Flutter 3.50, backups really off
2026-11-20 · accepted
Context: Flutter 3.50 refuses Gradle below 9.3.1 and the Kotlin Gradle plugin below 2.4.10. The Phase 1 scaffold from 3.44.6 had Gradle 9.1.0, AGP 9.0.1 and KGP 2.3.20, so `flutter build apk` failed after the FVM move (068). Separately, `mobile/CLAUDE.md` said Android backups were off, but the manifest did not set `allowBackup`, which defaults to on.
Decision: the versions a fresh Flutter 3.50 template pins, Gradle 9.5.0 (2026-04-28), AGP 9.3.1 (2026-07-23) and KGP 2.4.20 (2026-09-07). The manifest sets `allowBackup="false"`, `fullBackupContent="false"` and data extraction rules that exclude every domain from cloud backup and device transfer.
Consequences: A debug APK builds again and its merged manifest was checked with `aapt2`. The redirect scheme is a manifest placeholder per build type, debug gets `cloud.r16a.stream.dev`, profile and release get `cloud.r16a.stream`, and iOS sets the same through `R16A_OIDC_SCHEME` in the xcconfig files.

### 074 No group names in tokens, `profile` is not requested
2026-11-20 · accepted (found in the first real login)
Context: Authentik's default `profile` mapping returns `name`, `given_name`, `preferred_username`, `nickname` and `groups`, the names of every group the user is in, including groups of other applications such as `authentik Admins`. The design (C3, 022) meant the app to see only the two booleans. The backend needs only a display name from `profile`.
Verified in the 2026.2.1 source: claims come only from mappings whose scope the client requested and the provider has selected (`views/userinfo.py`), and a requested scope the provider lacks is silently dropped (`views/authorize.py`). `scopes_supported` in the discovery document is exactly the provider's selected mappings (`views/provider.py`).
Options: keep `profile` and ignore `groups` (the names still sit in every token, in the browser session store of Authentik and on the phone), a custom profile mapping without `groups`, or no `profile` at all.
Decision: the web and the app request `openid offline_access r16a-stream`. The two `r16a-stream` mappings also return `name` and `preferred_username`. No provider has `profile` or `email` selected, so even a modified client cannot get them, and the API warns at startup when a provider offers either.
Consequences: Tokens carry the subject, the display name, the username and two booleans, nothing about other groups or applications. A provider that still has `profile` selected keeps working, it only logs the warning.

### 075 Fixes from the Phase 2 security review
2026-11-20 · accepted
The security-reviewer agent reviewed all auth code on 2026-11-20. It found the backend's token, cookie, CSRF and flow handling sound and raised these, all fixed:
- **High, CI accounts reached production.** The CI users were in the production groups and the mapping was shared, so a leaked password-only CI admin account was a production admin. Fixed by separate CI groups, a dev-only mapping and dev-only bindings (051, SPEC 6.7, OPERATING 3.2).
- **Medium, revocation did not survive a refresh on mobile.** `iat` was compared with `sessions_revoked_at`, and the app's refresh produced a newer `iat`. Now `auth_time` is compared, which Authentik copies unchanged into refreshed tokens (verified in `views/token.py`), so revoking forces a new interactive sign-in. A token without `auth_time` is refused once the user has a revocation.
- **Medium, redirect interception on Android.** Any app can register the custom scheme and, with silent consent and a shared Custom Tab session, start its own flow and receive a code. Fixed for now with an authorization flow that always asks for consent on the mobile providers (the default explicit flow remembers consent for 4 weeks, verified in the blueprint). A verified Android App Link as redirect is the stronger fix, proposed for Phase 9 when the production host serves `assetlinks.json`.
- **Medium, pool exhaustion during an outage.** The refresh held a row lock and a pooled connection across up to two Authentik timeouts, and every stale session retried. Replaced by a 30 second refresh lease taken in one `UPDATE` (migration `20261120000002`), the Authentik call outside any transaction, and a 30 second per-provider fail-fast after an outage.
- **Low, a cancelled request could lose the rotated refresh token.** The refresh and its write run in a spawned task that the request awaits.
- **Low, a refresh in flight could undo a mobile sign-out.** Sign-out waits for it, and a generation counter stops a late refresh from writing, its new token is revoked.
- **Low, operator errors signed everyone out.** 429, `invalid_client`, non-JSON error pages and an unknown signing key are now outages, not refusals. Only `invalid_grant`, a lost claim and a changed subject end a session.
- **Low, rate limit key and flow table.** IPv6 clients are limited per /64, at most 10 000 login flows live at once, and production refuses an empty `R16A_TRUSTED_PROXY_CIDRS`.
- **Informational, all taken:** the callback query no longer prints its code in `Debug`, discovery endpoints must be on the issuer's host, the app sends the bearer token only to the API's own origin (presigned URLs must never get it), and release builds refuse non-https API or Authentik URLs.
Consequences: Tests cover each fix (`auth_web.rs`, `auth_bearer.rs`, mobile `token_manager_test.dart`, `auth_interceptor_test.dart`, `config_test.dart`). Mobile users see a consent screen on every sign-in, which with 30-day refresh tokens is rare.

### 076 Catalog in two parts, images in Phase 3b
2026-11-20 · accepted 2026-11-21, both parts built (078)
Context: Phase 3 is the catalog and admin content. Images (posters, backdrops, stills) need the uploads table, the job queue and a worker job type, which SPEC builds for transcoding in Phase 4. Building those parts twice would split one design.
Decision: Phase 3a delivers the catalog schema (with the image tables and the worker grants), browsing, search, the title page, admin editing with audit, the TMDB lookup, the web screens and the generated mobile models. Phase 3b adds the upload and job tables, TMDB image download into the uploads bucket, the worker image pipeline (SPEC 8.2), presigned image URLs per UTC day (8.3), and `seed --bench` with `docs/perf/queries.md` (4.10). Image fields in the API are null until then.
Consequences: The API shape does not change in 3b, `poster`, `backdrop` and `still` are already in the schema as nullable.

### 077 Phase 3a implementation choices
2026-11-20 · accepted 2026-11-21
- **Admin routes added to SPEC 5.2.** `GET /admin/titles`, `GET /admin/titles/{id}` (the editor needs drafts, which user-facing routes never show) and `POST /admin/genres` (manual titles need genres). Episode routes are `POST /admin/seasons/{id}/episodes`, `PATCH|DELETE /admin/episodes/{id}`.
- **The TMDB draft is a `TitleInput`.** The lookup returns exactly the create body, so the admin edits and posts it back. Genres travel as `newGenres` with their TMDB id and are created on save. No genres are seeded, TMDB genre ids would have come from memory.
- **Body limits.** The global `RequestBodyLimitLayer` is now the 2 MiB ceiling of the largest route, and `DefaultBodyLimit` stays 16 KiB for every route except `POST /admin/titles` and `POST /admin/titles/{id}/seasons`, which take 2 MiB. Tested with a 20 KiB patch (413) and a 400 episode show (201).
- **Search.** `websearch_to_tsquery('simple', f_unaccent(q))` or trigram similarity on the unaccented name, rank is `ts_rank + similarity` as float4, keyset on `(rank, id)`. Finds partial words and typos ("brakng bad").
- **Cursors** are base64url JSON of the sort key and id, not signed. They only move the caller through their own result set, and every query still carries its filters and `status = 'published'`.
- **Entity tags.** Catalog reads are `private, no-cache` with a weak tag over the body, `If-None-Match` answers 304.
- **OpenAPI query parameters.** utoipa declared every `IntoParams` struct as path parameters, also the Phase 2 login and callback queries. Fixed with `#[into_params(parameter_in = Query)]`, and `tests/openapi.rs` now fails on any path parameter without a placeholder.
- **Mobile models with `swagger_parser` 2.1.1, plain serializer.** 2.2.0, the approved version, was 6 days old (064). Version 2 writes plain `fromJson` and `toJson`, so `json_serializable`, `build_runner` and `json_annotation` from section B are not needed and not added, one package less ships in the app. `operation_filters.include_paths: []` generates models only, dio stays the client. Generated code is excluded from `flutter analyze`.
- **Web forms without `@angular/forms`.** Native inputs bound to signals, the admin editor is small. Sending only changed fields keeps PATCH semantics exact.
Consequences: Section B is updated accordingly.

### 078 Image pipeline and query tuning in Phase 3b
2026-11-20 · accepted 2026-11-21, including the move of manual image upload and episode stills to Phase 4
- **Scope.** TMDB posters and backdrops, the `jobs` table (without its variant column, added with `media_variants` in Phase 4), the worker image pipeline, presigned image URLs, and `seed --bench`. Manual image uploads need the uploads table and its multipart flow and move to Phase 4, as do episode stills.
- **Encoder settings.** AVIF with `libaom-av1 -still-picture 1 -crf 30 -cpu-used 6 -pix_fmt yuv420p`, WebP with `libwebp -quality 80`, both `-map_metadata -1`. Checked on Debian trixie's ffmpeg 7.1.5, which has both encoders, so the DECISIONS 018 check is done.
- **Heights** are computed with ffmpeg's own rule for `scale=w:-2`, and `tests/images.rs` compares them with the real output.
- **Placeholder colour** is the image averaged to one pixel by ffmpeg, stored in the design token HSL notation, and checked again by the web client before it becomes a style value.
- **Worker wake-up.** `LISTEN r16a_stream_jobs` with a 10 s poll as the fallback, and a reaper that requeues jobs without a heartbeat for 2 minutes, as SPEC 7.2 describes.
- **tokio `process` feature** enabled for spawning ffmpeg. A feature of an approved crate, no new dependency.
- **Search query rewritten.** The bench showed the CTE form could use neither GIN index (22 ms at 10 000 titles, linear). With the terms inline the planner combines both indexes, 0.04 ms for a rare word and 1.7 ms for a typo. The bench vacuums after seeding, because new GIN entries in the pending list hide the index from the planner. A future bulk import should end with `VACUUM (ANALYZE) titles` (docs/perf/queries.md).
- **Replaced images** keep their rows and objects until encode GC (Phase 4) removes images no title references.

### 079 Verified https redirect for mobile login, so consent can return to implicit
2026-11-21 · proposed, not implemented until approved
Context: The always-consent flow on the mobile providers (075) stops another Android app from catching the custom-scheme redirect, at the cost of one extra tap per sign-in. A verified Android App Link or iOS Universal Link on our own domain cannot be claimed by another app, the way Zaltys does it (`/.well-known/assetlinks.json` and `apple-app-site-association` served by the web container from `R16A_ANDROID_CERT_FINGERPRINTS` and `R16A_APPLE_APP_IDS`).
What was checked on 2026-11-21:
- **Consent is per provider, and Android and iOS share one.** Consent can go back to implicit only when the mobile provider accepts no custom-scheme redirect at all. If the custom scheme stays registered for either platform, a malicious Android app can still ask for it and get a silent code. So both platforms must move to https together.
- **Android with our current library works.** `flutter_appauth` 13 uses AppAuth-Android 0.11.1, which documents https redirects by overriding `RedirectUriReceiverActivity` in the manifest, and `android:autoVerify="true"` makes it an App Link. Caveat: AppAuth uses Custom Tabs, not Chrome's Auth Tab, and a Custom Tab does not always hand a server-side redirect over to an App Link. The AppAuth README recommends a fallback page at the redirect URL for that case, which would have to pass the code on, reopening the problem it solves.
- **iOS with our current library does not.** `flutter_appauth` 13 pins AppAuth-iOS 3.0.0, which builds `ASWebAuthenticationSession` with `callbackURLScheme` only (`OIDExternalUserAgentIOS.m`). An https callback needs the iOS 17.4 `ASWebAuthenticationSession.Callback.https(host:path:)` API, which AppAuth-iOS 3.0.0 does not call.
- **`flutter_web_auth_2` does both.** It uses Chrome's Auth Tab on Android (an https callback is returned to the calling app by the browser itself, no App Link handover needed, Custom Tab fallback on old Chrome) and the iOS 17.4 https callback on iOS. Zaltys uses it at 5.1.0, the newest release older than 7 days, 6.0.0 is 2 days old.
Options:
1. **Keep always-consent (today).** No code change. One extra tap per sign-in, and sign-ins are rare with 30-day refresh tokens.
2. **Android App Link with `flutter_appauth`, iOS keeps the custom scheme.** Does not allow dropping consent, see the first point. Not useful on its own.
3. **Switch mobile sign-in to `flutter_web_auth_2` with an https redirect `https://<host>/app/oauth2redirect` on both platforms.** The app builds the authorization URL with PKCE, state and nonce itself and redeems the code with dio at the token endpoint, about 150 lines, the same protocol code as the backend's (070). `flutter_appauth` goes, `flutter_web_auth_2` 5.1.0 comes in (needs your approval as a dependency). The mobile providers register only the https redirect and go back to implicit consent. The web container serves both well-known files from env, empty lists associate no app. iOS minimum becomes 17.4. The release build's signing fingerprint and the Apple app id become deployment config (OPERATING 9).
Trade-offs of option 3: one more piece of own protocol code to keep correct (mitigated by sharing the test approach of `tests/auth_*.rs`, a mock provider, and the checks the security review already applied), dependence on the browser honouring the verified association (Chrome 137+ for Auth Tab, otherwise Custom Tabs with the App Link), debug builds against a local API cannot use a verified link, so the dev provider keeps the custom scheme and always-consent, which is acceptable because no production tokens are involved, and an iOS 17.4 floor.
Recommendation: option 3, built in Phase 9 together with the production host and signing keys, because the well-known files need the real domain and the release fingerprint to be verifiable at all. Until then option 1 stays.

### 080 Variants with uploads in Phase 4
2026-11-21 · proposed
Context: The phase plan lists variants under Phase 3. A variant is one language of a movie or an episode, and its row (`media_variants`) points at its current transcode attempt, whose table, status machine and upload belong to Phase 4. In Phase 3 a variant could only ever be `awaiting_upload`.
Decision proposed: build variants (admin create and delete, the title page languages, the language filter maintenance) together with uploads and transcoding in Phase 4, as their first step. Phase 3 is otherwise complete.
Consequences: The admin can create titles, seasons and episodes now, and adds languages when uploading in Phase 4.
Status 2026-11-21: built that way in Phase 4 while the proposal awaits approval. Undoing it would only move the same code back into the Phase 3 label.

### 081 Upload, transcode and storage as built in Phase 4
2026-11-21 · proposed, part of the Phase 4 review
- **One decode, sprites included.** The encode feeds every rung, the audio rendition and the scrub sprites from one decoded stream (`split` into the rungs plus `fps=1/10,scale=160:-2,tile=10x10`). SPEC 7.3 step 6 described a second pass from the 360p output. One pass saves a full decode and keeps the HLS demuxer out of the format whitelist. The frames are the same.
- **Output layout** follows SPEC 10.2 exactly: `v<height>/`, `a/` for audio, `scrub/` with `sprite_<n>.webp` and `scrub.vtt`, whose cues name the sheets relative to it.
- **rustix 1.1.6** added (section B lists it) for `statvfs` and killing an encoder's process group. Published 2026-10-27, older than 7 days. Features `std`, `fs`, `process` only.
- **Reservations follow the job.** A transcode reserves source plus ladder on upload start (7.10). The reservation ends with the job, whether it succeeds, fails for good or is cancelled, including a cancel of a queued job by the API. A retry of a failed transcode and a re-transcode reserve the ladder again through admission.
- **The reaper** marks running attempts of requeued jobs `abandoned` and ends a job that was `cancelling` as `cancelled`, not queued.
- **Switch checks ownership.** The switch transaction re-reads the job under lock and only proceeds while this worker still owns it and no cancel was asked. A worker that lost its job to the reaper ends without touching the variant.
- **Retention re-check under lock.** Retention re-checks each candidate with the variant row locked, the lock re-transcode also takes, so a re-transcode queued meanwhile cannot lose its source. The lock is held across one Garage delete.
- **Manual images** start as one-part multipart uploads, remember their target (poster, backdrop or still, title or episode) on the upload row (migration `20261121000002`), and are attached when the upload completes. An abandoned image upload removes its pending image row.
- **Staged subtitles** are deleted by the API after the worker promoted or refused the track, tracked in `subtitle_tracks.staged_removed_at` (migration `20261121000003`), because only the API key may write the uploads bucket.
- **Episodes** now carry their still and the languages with a ready variant. Deleting a variant removes a title language no other ready variant has.
- **Hardware encoders** have pinned argument vectors and a one-frame test at start with x264 fallback. The quality settings (`-global_quality 23`) are placeholders until the ThinkStation measurements of SPEC 7.5, which are the owner's to run.
- **Mobile language names** are autonyms (`Italiano`), because Flutter has no counterpart of `Intl.DisplayNames`. The web uses names in the UI language.
- **Encode GC** walks the media bucket with delimiter listings one level at a time and never deletes a key it cannot parse.

---

## B. Dependencies (versions verified 2026-11-16)

"Brief" means named in the brief. "Needs approval" means I propose it and will not add it before you approve. **Status 2026-11-18:** every row is approved (060). The "needs approval" labels below are kept as the record of what the brief did not name. Constraints from 060 apply.

### B.1 Backend (crates.io)

| Crate | Version | Status | Why |
|---|---|---|---|
| axum | 0.8.9 | brief | HTTP framework |
| tokio | 1.54.1 | brief | runtime |
| sqlx | 0.9.0 | brief | features `postgres`, `runtime-tokio`, `tls-rustls-ring-webpki`, `macros`, `migrate`, `uuid`, `time`, `json` |
| tower-http | 0.7.1 | brief | trace, timeout, limit, compression-br and gzip, set-header, request-id, catch-panic, sensitive-headers |
| tracing | 0.1.44 | brief | logging |
| tracing-subscriber | 0.3.23 | needs approval | JSON log output, part of tracing |
| rusty-s3 | 0.11.0 | brief | SigV4 signing and presigning, `sign_with_time`. Sans-IO, so it needs an HTTP client. 0.12.0 appeared on 2026-11-18, reviewed in Phase 4 (064) |
| rustls | 0.23.46 (resolved) | approved (065) | direct only to install ring as the process crypto provider |
| thiserror | 2.0.21 | brief | typed errors |
| utoipa | 6.0.0 | brief ("or equivalent") | OpenAPI |
| tower | 0.5.3 | needs approval | middleware traits axum builds on |
| serde, serde_json | 1.0.229, 1.0.154 | needs approval | serialisation |
| reqwest | 0.13.5 | approved (060) | HTTP client for Garage, TMDB and Authentik, `default-features = false`, rustls only, streaming bodies. `openssl-sys` banned in `deny.toml` |
| jsonwebtoken | 12.0.0 | approved (021, 070) | verification of web ID tokens and mobile access tokens, `default-features = false`, `rust_crypto` |
| rsa | 0.10.0 | approved 2026-11-20, dev only (070) | generates the mock Authentik's signing key in tests |
| chacha20poly1305 | 0.11.0 | needs approval | sealing refresh and ID tokens at rest (020) |
| hmac, sha2, subtle | 0.13.0, 0.11.1, 2.6.1 | needs approval | playback token MAC, token hashing, constant-time compare |
| rand | 0.10.3 | needs approval | OS RNG (`SysRng`) for session tokens, state, nonces. 0.10.4 is younger than 7 days (072) |
| secrecy | 0.10.3 | needs approval | secrets in config never print |
| governor | 0.10.4 | needs approval | rate limiting (030) |
| uuid | 1.29.2 | needs approval | ids |
| time | 0.3.55 | needs approval | timestamps (sqlx has no `jiff` feature) |
| jiff | 0.2.46 (resolved) | approved | rusty-s3's time type, used only at the presign boundary |
| base64 | 0.23.1 | needs approval | tokens and cursors |
| url | 2.5.8 | needs approval | safe URL building, `serde` feature for the discovery document |
| bytes, futures-util, tokio-util | 1.12.1, 0.3.34, 0.7.20 | needs approval | streaming bodies to and from Garage |
| rustix | 1.1.6 | added 2026-11-21 (081), features `std`, `fs`, `process` | `statvfs` for the scratch space check, process group kill |
| dotenvy | 0.15.7 | needs approval | `backend/.env` in debug builds only, as Zaltys |
| tempfile | 3.27.0 | needs approval, dev only | test scratch dirs |

Tools: `sqlx-cli` 0.9.0, `cargo-deny` 0.21.0, `cargo-audit` 0.23.0, `cargo-chef` 0.1.78.

### B.2 Web (npm)

| Package | Version | Status | Why |
|---|---|---|---|
| @angular/* (core, common, router, platform-browser, forms) | 22.2.6 | brief | framework |
| @angular/cli, @angular/build | 22.2.7 | brief | build, dev server, Vitest runner |
| typescript | 6.0.3 | brief (implied) | Angular 22.2 requires `>=6.0 <6.1` |
| hls.js | 1.7.4 | brief | player |
| @jsverse/transloco | 8.4.0 | brief | i18n |
| rxjs | as required by Angular | brief (implied) | |
| vitest | 5.1.0 | brief (implied by Angular's runner) | unit tests, peer range `^4.0.8 \|\| ^5.0.0` |
| jsdom | 31.0.1 | needs approval, dev | test DOM |
| @playwright/test | 1.65.0 | brief | E2E, CSP and perf checks, Chromium build 1254 (072) |
| eslint, angular-eslint, typescript-eslint | 10.14.0, 22.6.0, 8.72.2 | approved, dev | lint, pinned back by the 7-day rule (072) |
| prettier | 3.10.1 | needs approval, dev | format |
| openapi-typescript | 7.13.0 | approved, dev | API types (042), runs on TypeScript 6 through an override (062) |
| @eslint/js | 10.0.1 | approved, dev | ESLint's own recommended rules, required by the flat config |
| tslib | 2.9.1 | brief (implied by Angular) | |
| @fontsource-variable/inter | 5.3.0 | needs approval | self-hosted font, no CDN under the CSP |

Node 26.13.1 LTS.

### B.3 Mobile (pub.dev)

| Package | Version | Status | Why |
|---|---|---|---|
| flutter_riverpod | 3.5.0 | brief | state |
| go_router | 18.1.4 | brief | routing |
| dio | 5.11.1 | brief | HTTP |
| video_player | 2.14.1 | brief | player |
| flutter_secure_storage | 11.2.0 | brief | token storage |
| intl, flutter_localizations | 0.20.3, SDK | brief (gen-l10n) | i18n, `intl` pinned to what the SDK's `flutter_localizations` pins |
| flutter_appauth | 13.0.0 | needs approval | native authorization code with PKCE in the system browser |
| cached_network_image | 4.0.7 | approved, added in Phase 3 | disk image cache keyed on the object key, decode at display size (043) |
| shared_preferences | 2.5.6 | needs approval | theme, UI language, non-secret prefs |
| wakelock_plus | 1.8.2 | needs approval | keep the screen on while playing |
| material_ui, cupertino_ui | 1.11.0, 1.1.6 | approved 2026-11-20 (069) | Material and Cupertino widgets and their localization delegates, moved out of the SDK in Flutter 3.47 |
| flutter_lints | 6.0.0 | needs approval, dev | analysis rules (Zaltys uses it) |
| mocktail | 1.0.5 | needs approval, dev | test doubles |
| swagger_parser | 2.1.1 | **approved** 2026-11-18, dev only | model generation from OpenAPI with the plain serializer, models only, generated code committed. 2.2.0 was younger than 7 days (077) |
| json_serializable, build_runner, json_annotation | not used | approved, not added | not needed, `swagger_parser` 2 generates plain codecs (077) |

Flutter 3.50.0, Dart 3.14.0, through FVM 4.3.2 (068).

### B.4 Images and tools

| Image or tool | Version | Status | Notes |
|---|---|---|---|
| postgres | 15.20 (`postgres:15`, digest pinned in CI) | brief | |
| dxflrs/garage | v1.0.1 | brief | never `latest` |
| rust (build stage) | 1.100.0-slim-trixie | proposed | cargo-chef layers |
| gcr.io/distroless/cc-debian13 | digest pinned | proposed | api runtime, nonroot |
| debian:trixie-slim | digest pinned | proposed | worker runtime with ffmpeg 7.1.5 from Debian trixie |
| nginxinc/nginx-unprivileged | 1.31.7-alpine | proposed | web runtime, Zaltys uses the same family |
| traefik (test only) | v3.6.7, digest pinned | accepted | presign and credential-stripping test, pinned to the cluster baseline (054) |
| traefik/whoami (test only) | v1.12.0, digest pinned | accepted | header echo standing in for Garage, proves `Cookie` and `Authorization` are stripped |
| curlimages/curl | 8.23.0, digest pinned | accepted | runs the Garage init, because the Garage image has no shell |
| mcr.microsoft.com/playwright | v1.65.0-noble, digest pinned | accepted | CI browser for the CSP check, matches @playwright/test |
| aquasec/trivy | 0.76.2 | proposed | image and filesystem scans |
| kubeconform | pinned in Phase 1 | proposed | manifest validation, as Zaltys |
| Gradle, AGP, Kotlin Gradle plugin | 9.5.0, 9.3.1, 2.4.20 | accepted (073) | Android build of the app, as the Flutter 3.50 template |
| FVM | 4.3.2, standalone binary verified by SHA-256 | accepted (068) | per-repository Flutter SDK from `mobile/.fvmrc`, locally and in CI |
| OWASP Dependency-Check action | pinned by SHA in Phase 1 | brief | |

---

## C. Re-check after upgrades

**Garage** (any version change, including 1.0.1 to 1.3.1 or 2.x)
- Path-style requests still accepted, `root_domain` semantics unchanged.
- Presigned query auth with a Host that differs from Garage's own address, signing time in the past (hour-floored), expiry up to 24 hours.
- Multipart: part size limits, ListParts paging, ETag format returned to the browser, Complete with 1 229 parts.
- Stored `Cache-Control` and `Content-Type` returned on GET.
- Key permission model (read, write, owner per bucket) and the two 403 negative tests.
- Behaviour when one of two nodes is down: reads succeed, writes fail fast rather than hang.
- `tests/garage_compat.rs` and `tests/garage_degraded.rs` pass against the new image.

**Garage metrics**: names and labels of `garage_local_disk_avail`, `garage_local_disk_total`, `garage_replication_factor`, `cluster_storage_nodes`, `cluster_storage_nodes_ok`, and that `metrics_token` still guards them separately from the admin token.

**Traefik** (including the 3.6.7 to 3.7 alignment, entry 054): matcher syntax (`QueryRegexp`, `HeaderRegexp`, `PathRegexp`), per-router access log switch, header deletion by empty value, Retry middleware semantics and which methods it retries, `ServersTransport` options, health check support for selector-less Services, `websecure` timeouts. `tests/traefik_presign.rs` runs against the new version first.

**OIDC client** (entries 070, 074, 075, on any Authentik or `jsonwebtoken` change): `scopes_supported` still lists exactly the selected mappings, a requested scope the provider lacks is still dropped silently, `auth_time` is still carried unchanged through refresh, the default `profile` mapping's claims, discovery document fields and issuer format with trailing slash, JWKS `kid` and `alg` on every key, token endpoint error body `{"error":"invalid_grant"}` for refused codes and refresh tokens, that a refresh response always carries a new `id_token` and `refresh_token`, that the revocation endpoint accepts `client_secret_basic`, that `sub` with hashed user ID is still `user.uid` (071). `tests/auth_web.rs` and `tests/auth_bearer.rs` encode these assumptions in the mock.

**Authentik** (entry 049, verified on 2026.2.1): claims from scope mappings in access tokens and their dependence on "Include claims in id_token", whether code and refresh grants evaluate application policies, whether `request.user` in a scope mapping is the token user on refresh, refresh token rotation and reuse detection, whether end-session honours `id_token_hint` and `post_logout_redirect_uri` (if it does, return to the app after logout), `is_active` checks on refresh, external user type in OAuth flows, the MFA stage binding policy for the CI users.

**ffmpeg**: argument vectors snapshot tests, `-format_whitelist` and `-protocol_whitelist` behaviour, availability of `libx264`, `libaom-av1`, `libwebp`, `h264_qsv`, `h264_vaapi` in the Debian build.

**Angular**: TypeScript range, `ngCspNonce` handling, Trusted Types policy names, budgets semantics.
