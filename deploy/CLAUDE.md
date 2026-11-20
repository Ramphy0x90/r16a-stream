# deploy

**Phase 2 done** for `compose/` (2026-11-20). `k8s/` is built in Phase 9.

How r16a-stream runs outside a developer machine. `compose/` is for local work and prod-like runs, `k8s/` holds the production manifests for the r16a cluster. Deployment detail in `docs/SPEC.md` sections 18 and 19, operator duties in `docs/OPERATING.md`.

## Rules specific to this folder

- Never write a secret value into a tracked file. `compose/.env` and `k8s/secret.yaml` are git-ignored. Only `.env.example` and `secret.example.yaml` are tracked. The dev-only Garage keys and the dev session key in the dev compose file are the exception, the backend refuses them outside development. The OIDC client secret is never in a tracked file, not even for dev.
- The API needs `R16A_OIDC_ISSUER`, `R16A_OIDC_WEB_CLIENT_ID`, `R16A_OIDC_WEB_CLIENT_SECRET`, `R16A_OIDC_MOBILE_ISSUER`, `R16A_OIDC_MOBILE_CLIENT_ID` and `R16A_SESSION_KEY` (SPEC 17). Without them it refuses to start. It starts while Authentik is unreachable, login answers 503 until discovery works.
- App containers run non-root, read-only root filesystem, all capabilities dropped, `allowPrivilegeEscalation: false`, seccomp `RuntimeDefault`, no service account token. Writable paths get an `emptyDir` or tmpfs, never a relaxed context.
- Every container sets CPU and memory requests and limits and HTTP probes on `/healthz` and `/readyz`.
- One file per component in `k8s/`, Deployment and Service together. No kustomize, no Helm, no per-resource sprawl.
- The worker is the only workload with a NetworkPolicy, egress to Postgres pods (`app=postgres` in `r16a-databases`), Garage `192.168.1.111` and `192.168.1.112` on TCP 3900 only, and kube-dns. Never open 3901 (Garage RPC) or 3903 (admin). Do not add policies elsewhere without a DECISIONS entry.
- The worker is placed by the node label `r16a.cloud/transcode=true`, never by hostname. Scratch is a local PV on that node, 200 Gi by default.
- Three database roles everywhere: `r16a_stream_owner`, `r16a_stream_app`, `r16a_stream_worker`. `init-roles.sh`, `provision.sql` and CI create all three.
- Only features of Traefik 3.6.7 in manifests until the cluster is aligned (DECISIONS 054). The retry middleware goes on the media router only, never on uploads.
- Never use `latest` for Garage or Postgres. Garage is `dxflrs/garage:v1.0.1`, Postgres `postgres:15`.
- Validate manifests with kubeconform and the `k8s-manifest-hardening` checklist.

## Compose

- `docker-compose.dev.yml` (project `r16a-stream-dev`) always starts `db` on 127.0.0.1:5433, `garage` on 3900 and 3903, and `garage-init`. Profiles `backend`, `worker`, `web` add the app containers. Empty `R16A_DEV_*_PORT` values pick free ports, which CI uses.
- `test/traefik/` is the Traefik 3.6.7 harness that mirrors the IngressRoute with the file provider. Keep it in step with `k8s/ingress.yaml` once that exists.
- A container `r16a-stream-postgres-1` from the earlier attempt may still hold 5432 on the development machine. The user removes it themselves, agents never do.
- `garage/` holds `garage.toml` and `init.sh`. The init is idempotent and fails unless the API key gets 403 writing media and the worker key 403 writing uploads.
- `docker-compose.yml` (project `r16a-stream-local`) runs everything from the real Dockerfiles with secrets from `.env`. Garage gets its RPC secret and tokens from the environment. The worker is only on internal networks.
- `seed/` holds tiny sample clips for local testing.

## Kubernetes (planned)

- Namespace `r16a-stream`. `backend.yaml` (with `migrate` init container that alone gets `MIGRATE_DATABASE_URL`), `worker.yaml`, `web.yaml`, `ingress.yaml` (Ingress, bucket IngressRoute and middlewares, selector-less `garage` Service and EndpointSlice), `networkpolicy.yaml`, `provision.sql`, `secret.example.yaml`.
- Images `registry.r16a.cloud/r16a-stream-backend`, `-worker`, `-web`, pull secret `registry-r16a-stream`, Secret `r16a-stream-secret`.
- CI applies manifests by name, never the folder, because `secret.example.yaml` sits next to them.

## Gotchas (from Zaltys)

- In `secret.yaml` under `data:`, encode with `printf '%s' "$value" | base64 -w0`. A trailing newline breaks database URLs.
- Changing the Secret does not restart pods. Use `kubectl rollout restart`.
- `kubectl logs deploy/...` picks an arbitrary pod. During a bad rollout, read the newest pod.
