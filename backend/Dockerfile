# syntax=docker/dockerfile:1.7
# Two images from one build, selected with --target.
#   api     distroless, runs `api serve`, the migrate init container runs `api migrate`
#   worker  Debian slim with ffmpeg, runs `worker run`
# Build context is the backend directory. sqlx checks queries against the committed .sqlx
# cache, so no database is needed at build time.
#
# Base images are pinned by digest. Refresh with `docker buildx imagetools inspect <image:tag>`.

FROM rust:1.100.0-slim-trixie@sha256:6f14b2b4b278ec0d50aa1d69541c304ce0f95e0f6268487aa9924af1c2e9aebe AS chef
# The image already holds this toolchain. Naming it keeps rust-toolchain.toml from asking
# rustup to download the clippy and rustfmt components a build does not need.
ENV RUSTUP_TOOLCHAIN=1.100.0 CARGO_NET_RETRY=5
RUN cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
ENV SQLX_OFFLINE=true
# Dependency layer, rebuilt only when Cargo.toml or Cargo.lock change.
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin api --bin worker

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:cb87acc574abe719b6e5c547384defa2f7b1f24af9605d3f618cf852495d8be2 AS api
COPY --from=build /src/target/release/api /api
USER 65532:65532
ENV R16A_HTTP_BIND=0.0.0.0:8081
EXPOSE 8081
ENTRYPOINT ["/api"]
CMD ["serve"]

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2 AS worker
# ffmpeg from Debian trixie (7.1), the version the pipeline is written and tested against.
# Security fixes newer than the pinned base are pulled with upgrade. CI passes the commit SHA as
# APT_REFRESH, so this layer is rebuilt on every build instead of reusing a stale cache.
ARG APT_REFRESH=unset
RUN echo "apt refresh ${APT_REFRESH}" \
    && apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && apt-get install -y --no-install-recommends ffmpeg ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 65532 --user-group --no-create-home --shell /usr/sbin/nologin worker \
    && install -d -o 65532 -g 65532 -m 0700 /scratch
COPY --from=build /src/target/release/worker /worker
USER 65532:65532
# The root filesystem is read-only in compose and Kubernetes. ffmpeg and fontconfig write
# their caches under HOME, so it points at the tmpfs mounted on /tmp.
ENV HOME=/tmp \
    R16A_WORKER_HEALTH_BIND=0.0.0.0:8082 \
    R16A_WORKER_SCRATCH_DIR=/scratch \
    R16A_FFMPEG_PATH=/usr/bin/ffmpeg \
    R16A_FFPROBE_PATH=/usr/bin/ffprobe
EXPOSE 8082
ENTRYPOINT ["/worker"]
CMD ["run"]
