# Operating r16a-stream

Runbook for deploying and running r16a-stream on the r16a cluster. Skeleton, updated 2026-11-18 after the Phase 0 approval. Sections marked **TODO (Phase n)** are completed in that phase. Facts already decided are written out now so nothing gets lost.

## 1. Overview

- Namespace `r16a-stream`. Deployments `r16a-stream-backend` (with `migrate` init container), `r16a-stream-worker`, `r16a-stream-web`.
- Images in `registry.r16a.cloud`, pull secret `registry-r16a-stream`, Secret `r16a-stream-secret`.
- Database `r16a_stream` on the shared PostgreSQL 15 in `r16a-databases`, roles `r16a_stream_owner`, `r16a_stream_app` and `r16a_stream_worker`.
- Garage v1.0.1 on `192.168.1.111` and `192.168.1.112`, buckets `r16a-stream-media` and `r16a-stream-uploads`, keys `r16a-stream-api` and `r16a-stream-worker`, plus the read-only `metrics_token` for the API.
- Authentik 2026.2.1, applications `r16a-stream`, `r16a-stream-mobile`, `r16a-stream-dev`, `r16a-stream-mobile-dev`, groups `r16a-stream-users` and `r16a-stream-admins`.
- Traefik baseline 3.6.7 (the cluster also runs 3.7.13 until aligned).

## 2. Secrets

**TODO (Phase 9)** full procedure. Keys of `r16a-stream-secret`:

| Key | Generate with | Consumer |
|---|---|---|
| `DATABASE_URL` | app role password from `provision.sql` | backend |
| `WORKER_DATABASE_URL` | worker role password from `provision.sql` | worker (`R16A_WORKER_DATABASE_URL`) |
| `MIGRATE_DATABASE_URL` | owner role password | `migrate` init container only |
| `OIDC_WEB_CLIENT_SECRET` | Authentik provider `r16a-stream` | backend |
| `SESSION_KEY` | `openssl rand -base64 32` | backend |
| `PLAYBACK_TOKEN_KEY` | `openssl rand -base64 32` | backend |
| `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | `garage key create r16a-stream-api` | backend |
| `WORKER_S3_ACCESS_KEY_ID`, `WORKER_S3_SECRET_ACCESS_KEY` | `garage key create r16a-stream-worker` | worker |
| `GARAGE_METRICS_TOKEN` | the `metrics_token` from `garage.toml` (5.1) | backend |
| `TMDB_TOKEN` | TMDB account, API read access token | backend |

Rotation: `SESSION_KEY` and `PLAYBACK_TOKEN_KEY` have `_PREVIOUS` companions. Move the current value to `_PREVIOUS`, set a new current one, restart, and remove `_PREVIOUS` after 30 days (sessions) or 1 day (playback). Base64 values in `secret.yaml` under `data:` must be encoded with `printf '%s' "$v" | base64 -w0` (Zaltys gotcha).

GitHub secrets for CI: `R16A_DEV_OIDC_CLIENT_SECRET`, `R16A_CI_USER_PASSWORD`, `R16A_CI_ADMIN_PASSWORD`, plus the registry and cluster ones as in Zaltys.

## 3. Authentik

### 3.1 Applications, providers and logout flows

Exact settings are in SPEC 6.1. The steps below use the setting names from SPEC 6.1, which were checked against the 2026.2.1 source. The labels in the admin UI were not checked on a live instance yet, so confirm them on the first run and correct this text where they differ.

**Once for all providers**
1. Directory, Groups, Create `r16a-stream-users` and `r16a-stream-admins`. Add every admin to both.
2. Customization, Property Mappings, Create, **Scope Mapping**. Name `r16a-stream: access claims`, scope name `r16a-stream`, expression from SPEC 6.1.

**For each of the four providers** (`r16a-stream`, `r16a-stream-mobile`, `r16a-stream-dev`, `r16a-stream-mobile-dev`), Applications, Providers, Create, **OAuth2/OpenID Provider**:
1. Name equal to the slug. Authorization flow `default-provider-authorization-implicit-consent`. Invalidation flow, the provider's own logout flow (below, create the flows first or come back to this field).
2. Client type **Confidential** for `r16a-stream` and `r16a-stream-dev`, **Public** for the two mobile ones. Client ID equal to the slug, so `R16A_OIDC_WEB_CLIENT_ID` and the app's built-in ids match.
3. Redirect URIs, strict, exactly those in SPEC 6.1 for that provider. Nothing else, no wildcards.
4. Signing key, an RSA certificate (for example `authentik Self-signed Certificate`). Without one Authentik signs with HS256 and the API refuses every token.
5. Advanced protocol settings: access code validity `minutes=1`, access token validity `minutes=5`, refresh token validity `days=30`, refresh token threshold `seconds=0`, scopes `openid`, `profile`, `offline_access` and `r16a-stream` (the mapping from step 2 above), subject mode "Based on the User's hashed ID", include claims in id_token **on**, issuer mode "Each provider has a different issuer". The hashed ID subject is the same for one person across all providers of this Authentik, which is what lets the web and the app share an account (DECISIONS 071).
6. Applications, Create, name and slug equal to the provider name, provider the one just created. Launch URL as in SPEC 6.1. Policy / Group / User Bindings, exactly one binding, group `r16a-stream-users`.
7. For the two confidential providers, copy the client secret into the API's secret store (`R16A_OIDC_WEB_CLIENT_SECRET`) and nowhere else. For the dev one that is your git-ignored `backend/.env` or `deploy/compose/.env`, and the GitHub secret `R16A_DEV_OIDC_CLIENT_SECRET`.

**API configuration** follows from the slugs. `R16A_OIDC_ISSUER` is `https://<authentik>/application/o/<web slug>/`, `R16A_OIDC_MOBILE_ISSUER` the same with the mobile slug, both on the same Authentik host, which the API checks at startup. The app gets the Authentik origin with `--dart-define=R16A_AUTHENTIK_URL=https://<authentik>`.

Checklist:
- both groups exist, admins are also members of `r16a-stream-users`,
- the scope mapping `r16a-stream: access claims` with the expression from SPEC 6.1, attached to all four providers,
- all four providers have a signing key selected (otherwise HS256 with the client secret), "Include claims in id_token" on, access token validity `minutes=5`, refresh token validity `days=30`,
- each application has exactly one group binding to `r16a-stream-users`,
- redirect URIs are strict and exact,
- a non-member test account is refused at Authentik.

**Logout flows** (why: Authentik 2026.2.1 ignores the redirect parameter of RP-initiated logout, so the redirect must come from the flow, and a provider-specific flow avoids touching the default flows that other apps use). Once:
1. Flows and Stages, Stages, Create, **User Logout Stage**, name `r16a-stream-user-logout`.
2. For each of the four providers, Create, **Redirect Stage**, mode Static, name and target from SPEC 6.1.1 (for example `r16a-stream-logout-redirect` to `https://stream.r16a.cloud/`).
3. For each provider, Flows, Create, name and slug from SPEC 6.1.1, designation **Invalidation**, authentication **No requirement**. Bind `r16a-stream-user-logout` at order 10 and that provider's Redirect stage at order 20.
4. On each provider, set **Invalidation flow** to its new flow.
5. Check: sign in to the app, sign out, land on the app's start page signed out, and confirm the Authentik session is gone by opening Authentik in the same browser.

### 3.2 CI test users

Why: Playwright tests the real login. These two accounts have no MFA, so they are scoped as tightly as Authentik allows (threat T-AUTH-13).

**Which flow and stage the change touches.** The MFA exception is bound to the **Authenticator Validation** stage binding of the authentication flow those users sign in through:
- If the `r16a-stream-dev` provider has an "Authentication flow" set, that flow.
- Otherwise the brand's authentication flow: System, Brands, the brand serving your Authentik hostname, "Authentication flow". With defaults this is `default-authentication-flow`, whose MFA stage is `default-authentication-mfa-validation` at order 30.

**Every other application on that flow** must be checked, because step 4 below changes that binding for all of them. List them:
1. System, Brands: note the authentication flow of the brand (call it F).
2. Applications, Providers: open each provider. Every provider whose "Authentication flow" is empty or set to F uses F. Proxy, SAML, LDAP and RAC providers have the same field.
3. Or with an API token: `GET /api/v3/providers/all/?page_size=200` and read `authentication_flow` (null means the brand's flow), then `GET /api/v3/core/brands/` for the brand's `flow_authentication`.
Write the resulting application list here before applying the change: **to fill in** (needs your Authentik instance, which the agents cannot reach).

**Setup** (run once, in this order):
1. **Audit other applications.** In Authentik an application with no policy binding is open to every user. Make sure every application other than the r16a-stream ones has a binding (group or policy) that the CI users do not satisfy.
2. Create users `r16a-stream-ci-user` and `r16a-stream-ci-admin`, user type **external**. Add the first to `r16a-stream-users`, the second to `r16a-stream-users` and `r16a-stream-admins`. No other groups.
3. Set passwords of at least 32 random characters (`openssl rand -base64 32`). Store them in GitHub secrets `R16A_CI_USER_PASSWORD` and `R16A_CI_ADMIN_PASSWORD`, nowhere else.
4. Customization, Policies, Create, **Expression Policy** `r16a-stream-ci-skip-mfa` with the expression from SPEC 6.7. Then in flow F, Stage Bindings, the Authenticator Validation binding:
   - Edit the binding, set **Policy engine mode** to `all` (it is `any` by default). Leave "Evaluate when flow is planned" off and "Evaluate when stage is run" on.
   - Bind Policy, choose `r16a-stream-ci-skip-mfa`, order 20, **Negate** on, **Failure result** pass.
   Why both: with `any`, the existing passwordless policy already passes for every password login, so the stage would run regardless. With `all`, the stage runs when the existing policy passes and the user is not a CI user, which for every other user is exactly today's result.
5. **Check access.** For both users, Applications, each application, "Check access". Expected: allowed on `r16a-stream-dev` only.
6. **Manual MFA check, after every change to flow F**: in a private window, sign in as a normal user who has MFA and confirm the MFA prompt appears. Then sign in as `r16a-stream-ci-user` and confirm it does not. Record the date here. CI covers the second half on every run.

**Password rotation** (every 90 days, and immediately if a CI log or secret may have leaked):
1. Generate a new password with `openssl rand -base64 32`.
2. Set it in Authentik (Directory, Users, the user, Set password).
3. Update the GitHub secret with the same value.
4. Re-run the latest CI workflow and confirm the Playwright login passes.
5. In Authentik's event log, check that logins of the CI users came only from the CI runner.

The dev web client secret (`R16A_DEV_OIDC_CLIENT_SECRET`) rotates the same way, from the `r16a-stream-dev` provider page, followed by updating local `backend/.env` files.

## 4. Database

**TODO (Phase 9)** `provision.sql` run once as a superuser, the same way as Zaltys:

```
kubectl -n r16a-databases exec -i <postgres pod> -- psql -U postgres \
  -v owner_pw='...' -v app_pw='...' -v worker_pw='...' < deploy/k8s/provision.sql
```

It creates the database and the three roles. Table grants for the worker come with the migrations. Migrations run in the `migrate` init container on every rollout. They are forward only. A failed migration keeps the old pods running.

## 5. Garage

**TODO (Phase 9)** buckets, keys and permissions on the production cluster, mirroring `deploy/compose/garage/init.sh`.

Checks before the first deploy and after any rename:
- `root_domain` in `garage.toml` (`[s3_api]`) is **not** a suffix of the app host. With `root_domain = ".r16a.cloud"`, Garage would read `stream.r16a.cloud` as bucket `stream` and every presigned URL would fail.
- No bucket has website access or anonymous read.
- API key: read on media, read and write on uploads. Worker key: read on uploads, read and write on media. Test that the API key gets 403 writing media and the worker key 403 writing uploads.
- Both node IPs (`192.168.1.111`, `192.168.1.112`) are in the `garage` EndpointSlice, in `R16A_S3_INTERNAL_ENDPOINTS` and in the worker NetworkPolicy, on port 3900 only.

### 5.1 Admin port and metrics token

Why: the app needs free disk space per node to refuse uploads that would not fit and to warn before Garage fills up. Garage publishes it on `/metrics` of its admin port, guarded by `metrics_token`, which can only read metrics. The full `admin_token` could create keys and grant itself write access to media, so the app never gets it.

On each Garage VM, in `garage.toml`:

```toml
[admin]
api_bind_addr = "0.0.0.0:3903"
metrics_token = "<openssl rand -base64 32, same on both nodes>"
# admin_token stays as it is today, or unset. Never put it in r16a-stream-secret.
```

Then restart Garage on that node, one node at a time, and allow TCP 3903 in the VM firewall **from the cluster node IPs only** (Calico NATs pod traffic leaving the cluster to the node IP). Never route 3903 through Traefik. Port 3901 (RPC) stays closed to the cluster.

Check from a cluster node:
```
curl -s -H "Authorization: Bearer <metrics token>" http://192.168.1.111:3903/metrics | grep garage_local_disk_avail   # numbers
curl -s -o /dev/null -w '%{http_code}\n' http://192.168.1.111:3903/metrics                                             # 403
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer <metrics token>" http://192.168.1.111:3903/v1/key  # 403
```
The last line proves the metrics token opens no admin endpoint. `/health` answers without a token by design, that is expected. The same three checks run against the dev container in `tests/garage_compat.rs`.

**Until this section is done**, leave `R16A_GARAGE_METRICS_ENDPOINTS` empty. The storage page then says "capacity unknown" and uploads are admitted within the fixed budget `R16A_STORAGE_UNKNOWN_BUDGET_BYTES` (SPEC 7.10). Nothing breaks, but nothing stops Garage from filling up through many uploads in a row, so finish this before the library grows.

### 5.2 One node down, what users see

With replication 2, reads need one node and writes need both.

| Area | Behaviour |
|---|---|
| Browsing, title pages, lists, comments, requests, settings | normal, they only use Postgres |
| Posters and playback | work. The first read that hits the dead node waits for the 2 s dial timeout and is retried on the live node. Traefik's passive health check then skips the dead node for 30 s, so later reads are not delayed. Every 30 s one read re-tests the dead node and waits 2 s. The player's buffer usually hides it |
| Starting playback | normal, the API fails over to the live node itself |
| Uploads | paused. The admin area shows "storage degraded", new uploads and part URLs answer 503, a running upload waits and resumes |
| Transcodes, image and subtitle jobs | wait in the queue as `storage_degraded`, resume on their own |
| TMDB import with images, subtitle upload | answer 503 until both nodes are back |
| Source retention and encode GC | postponed, deletes are writes too |

Operator actions:
1. Optional for a long outage: remove the dead node from the EndpointSlice so Traefik stops re-testing it every 30 s: `kubectl -n r16a-stream edit endpointslice garage-manual` and set `conditions.serving: false` (or delete the entry) for that address.
2. Repair the node. Garage resyncs on its own when it returns.
3. Restore the EndpointSlice entry. Jobs and uploads continue without intervention.

After the cluster runs Traefik 3.7 everywhere, the active health check in DECISIONS 054 makes step 1 unnecessary.

**Replication is not a backup.** Replication factor 2 protects against one disk or server failing. It does not protect against deletion, a bad GC run, corruption that replicates, or losing both servers. See section 7.

**Upgrades.** DECISIONS 055 compares 1.3.1 and 2.x. Whatever the target, run `tests/garage_compat.rs`, `tests/garage_degraded.rs` and `tests/traefik_presign.rs` against the target image before upgrading the cluster, then go through DECISIONS section C.

### 5.3 Storage full

The admin area warns when logical free space minus reservations drops below `R16A_STORAGE_WARN_FREE_BYTES` (default 100 GB), refuses uploads that do not fit above `R16A_STORAGE_MIN_FREE_BYTES` (default 20 GB), and the worker holds outputs it cannot fit (`storage_full`). To free space, use the per-title storage list, delete variants or titles that are no longer needed (their encodes go after the 24 hour GC grace), or add capacity. **TODO (Phase 4)** step by step.

## 6. Traefik

**Cluster-wide change, needed before uploads work.** The `websecure` entrypoint needs `transport.respondingTimeouts.readTimeout: 300s`, because the 60 s default cuts off a 50 MiB part on a slow uplink. It affects every app on the cluster. Trade-off: slow-request (slowloris-style) clients can hold connections up to 5 minutes. Keep `writeTimeout` and `idleTimeout` at their defaults. **TODO (Phase 9)** the exact change for how Traefik is installed in the cluster (Helm values or static config) and whether per-IP connection limiting is wanted.

The manifests use only 3.6.7 features until the cluster is aligned. Access logs are off for the playback and bucket routers through per-router `observability.accessLogs: false`.

## 7. Backups

Not built yet. Design, to be built before the deployment phase. The target is chosen before Phase 9.

**What changed with source retention.** Sources are deleted after a verified transcode by default (SPEC 7.9). The encodes in the media bucket are then the only copy of the content, so the backup copies them instead of the sources.

- **Target**: any S3-compatible endpoint off the cluster, configurable (endpoint, bucket, region, key). Nothing in the design depends on a specific provider.
- **Database**: nightly `pg_dump --format=custom` of `r16a_stream` from a CronJob in `r16a-stream`, streamed to the target, 14 days retention through a lifecycle rule or a prune step.
- **Media bucket**: incremental copy of `v/`, `i/` and `s/` (everything except `_health/`) with `rclone copy` or equivalent, nightly. Objects are immutable per key, so each night copies only new keys. Deleted keys are kept on the target for 30 days before pruning, so a bad GC run or an accidental delete can be undone.
- **Uploads bucket**: `images/` originals always. `sources/` only while `R16A_SOURCE_RETENTION=keep`, otherwise only the short-lived sources that have not been transcoded yet.
- **Size**: roughly the size of the library, up to the 950 GB Garage holds. Size the target accordingly.
- **Secrets**: `secret.yaml`, the Garage `metrics_token` and the Authentik application settings are kept outside the cluster by you.
- **Restore order**: database first (so ids match), then the media bucket, then images. No re-transcode is needed, since the encodes come back as they were.
- **Manual restore test**, at least after the backup is first built and then every 6 months: restore the latest dump into a scratch database on `postgres:15`, copy one title's prefixes from the target into a scratch Garage (`dxflrs/garage:v1.0.1` in compose), point a local API at both, and play that title. Write the date and result here.

## 8. Users and data

- Access is managed only in Authentik by group membership.
- Without further action, removing someone from `r16a-stream-users` ends their access at their next token refresh, at most 5 minutes later, because the `r16a_stream_user` claim turns false. Authentik itself does not re-check the group on refresh, and refresh tokens live 30 days, so the steps below are what makes removal certain and immediate.

**Revoke a user's access now** (all steps, in this order):
1. **Authentik, stop new tokens.** Directory, Users, the user: remove them from `r16a-stream-users` (and `r16a-stream-admins`), or Deactivate if they should lose every application.
2. **Authentik, kill what is issued.** On the same user page, delete every entry under **Sessions**, **OAuth Refresh Tokens** and **OAuth Access Tokens**.
3. **r16a-stream, end live sessions.** Admin, Users, find the user, **Revoke sessions** (the admin screen arrives with the admin area, until then call `POST /api/v1/admin/users/{id}/revoke-sessions` as an admin). This deletes their web sessions and makes the API refuse every mobile access token issued before now (SPEC 6.8). It is audited.
4. Check: the user's browser gets the sign-in page on the next request, and their phone is signed out on its next call.

Step 3 alone ends sessions but a user still in the group can sign in again. Step 1 alone takes up to 5 minutes. Together they are immediate and final.
- **TODO (Phase 6)** deleting a user's data (`users` row cascades to sessions, lists, progress, reactions, comments, votes).

## 9. Rename checklist

When the final name or hostname is chosen, change all of these together.

| Where | What |
|---|---|
| `deploy/k8s/ingress.yaml` | host in the Ingress and in every IngressRoute rule, TLS secret name |
| `deploy/k8s/backend.yaml` | `R16A_PUBLIC_URL`, `R16A_S3_PUBLIC_ENDPOINT` |
| Authentik | redirect URIs of `r16a-stream`, application launch URLs |
| Garage | re-check `root_domain` is not a suffix of the new host |
| Cloudflare | DNS record, DNS-only |
| web nginx | nothing host-specific, the CSP is `'self'` |
| mobile | `R16A_API_BASE_URL` dart-define for release builds |
| mobile app id | `android/app/build.gradle.kts` `applicationId`, iOS `PRODUCT_BUNDLE_IDENTIFIER`, redirect schemes `cloud.r16a.stream` and `cloud.r16a.stream.dev` in the Android manifest placeholders and iOS `Info.plist`, and the redirect URIs of `r16a-stream-mobile` and `r16a-stream-mobile-dev` |
| docs | `README.md`, root `CLAUDE.md`, this file |

Code, env vars and resource names (`r16a-stream`, `R16A_`) stay as they are, as in Zaltys.

## 10. Worker node and scratch volume

Why a label: you have not chosen the node yet, and a label lets you move the worker by moving one label instead of editing manifests.

1. Pick a node with a fast local disk and room for the scratch volume (default 200 Gi).
2. Label it, and make sure no other node has the label, because the local PersistentVolume describes one disk on one node:
   ```
   kubectl label node <node> r16a.cloud/transcode=true
   kubectl get nodes -l r16a.cloud/transcode=true    # exactly one line
   ```
3. On that node, create the directory the PV points to, owned by the worker's UID (65532) so the non-root container can write:
   ```
   sudo mkdir -p /var/lib/r16a-stream/scratch
   sudo chown 65532:65532 /var/lib/r16a-stream/scratch
   sudo chmod 0700 /var/lib/r16a-stream/scratch
   ```
4. Apply `deploy/k8s/worker.yaml`. It contains the StorageClass `r16a-local-nvme`, the PV (`local.path: /var/lib/r16a-stream/scratch`, `nodeAffinity` on the label, 200 Gi) and the PVC. Check `kubectl -n r16a-stream get pvc r16a-stream-scratch` shows `Bound` once the worker is scheduled (`WaitForFirstConsumer` binds only then).
5. To change the size, edit the capacity on the PV and the request on the PVC together. A local PV cannot be resized in place, so delete the worker, the PVC and the PV, then re-apply. Scratch holds nothing that needs keeping.
6. To move the worker: delete the worker Deployment, the PVC and the PV, move the label, repeat steps 3 and 4 on the new node.

Concurrency: `R16A_WORKER_TRANSCODE_CONCURRENCY` (default 1). Each running job needs about 1.6x its source size on scratch, and the worker checks free space before starting one.

## 11. Joining the ThinkStation for Quick Sync

**TODO (Phase 4)**. x264 stays the default until this is done. Outline:
1. Install the same OS baseline and container runtime as the other nodes, join with `kubeadm join`.
2. If the worker should move there, move the `r16a.cloud/transcode=true` label (section 10).
3. Install the Intel GPU device plugin (operator or DaemonSet) so the node advertises `gpu.intel.com/i915`. Why: the plugin hands the render device to a pod without a privileged container or a hostPath.
4. Set `R16A_WORKER_ENCODER=qsv` (or `vaapi`) and add the `gpu.intel.com/i915: 1` request in `worker.yaml`.
5. Check `/readyz` details show the hardware encoder, not the x264 fallback.

## 12. Deploy and rollback

**TODO (Phase 9)**. First deploy, CI flow, `kubectl rollout undo`, the migration caveat (forward only, so a rollback past a migration needs a compatible schema).

## 13. Before production

Two infrastructure decisions are open. Decide both before the first production deploy, with the Phase 1 test results in hand.

| Decision | Today | If upgraded | What changes in r16a-stream | Evidence to look at |
|---|---|---|---|---|
| Traefik alignment | 3.6.7 and 3.7.13 mixed, manifests use 3.6.7 features only | all replicas on 3.7.x | the commented `healthCheck` block in `ingress.yaml` can be enabled (DECISIONS 054), removing the 2 s delays while a Garage node is down. Nothing else changes | `tests/traefik_presign.rs` against the target version |
| Garage version | 1.0.1 | 1.3.1 (low risk, in place, reversible within 1.x) or 2.x (one-way) | 1.3.1: nothing in our code. 2.x: capacity could come from a scoped admin token instead of metrics, compose init syntax, metric names re-checked (DECISIONS 055) | the invalid UTF-8 result in DECISIONS 055, `tests/garage_compat.rs` and `tests/garage_degraded.rs` against the target image |

Other items that must be done before production:
- OPERATING 5.1 (admin port, metrics token, firewall).
- OPERATING 6 (Traefik `websecure` read timeout 300 s).
- OPERATING 3.1 and 3.2 (providers, logout flows, CI users with the application list filled in).
- OPERATING 7 (backup target chosen, first backup and one restore test done).
- The npm override for openapi-typescript (DECISIONS 062) is removed if a release that officially supports TypeScript 6 exists and is at least 7 days old. If none exists yet, record the check and its date in DECISIONS 062.

## 14. Incidents

**TODO (Phase 9)**. Storage degraded (5.2), storage full (5.3), worker stuck, leaked playback URL (rotate `PLAYBACK_TOKEN_KEY`), leaked session key, compromised admin account, leaked CI password (3.2).
