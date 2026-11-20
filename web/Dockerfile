# syntax=docker/dockerfile:1.7
# Build context is the repository root, because the web build needs the npm workspace (root
# package.json and lockfile) and packages/design-tokens.
#
# Base images are pinned by digest. Refresh with `docker buildx imagetools inspect <image:tag>`.

FROM node:26.13.1-alpine@sha256:6bd9ab43c0ec0190ca7b2d193906cb8b7edf6e7f4afa43ddeccd6452710bffb5 AS build
WORKDIR /repo
# npm ci checks every workspace against the lockfile, so all workspace manifests come first.
COPY package.json package-lock.json .prettierrc ./
COPY web/package.json web/
COPY packages/design-tokens/package.json packages/design-tokens/
RUN npm ci --no-audit --no-fund --workspace web --include-workspace-root
COPY packages packages
COPY web web
WORKDIR /repo/web
RUN npm run build

FROM nginxinc/nginx-unprivileged:1.31.7-alpine@sha256:b5047b9b76845eac124358373c6852377c52e4b05834a47647e955dd91cb8f1e
# Pull Alpine security fixes newer than the base image. CI passes the commit SHA as
# APK_REFRESH so this layer is rebuilt on every build while the npm stage stays cached.
ARG APK_REFRESH=unset
USER root
RUN echo "apk refresh ${APK_REFRESH}" && apk upgrade --no-cache && apk del --no-cache curl
RUN rm -f /etc/nginx/conf.d/default.conf
COPY web/nginx/nginx.conf /etc/nginx/nginx.conf
COPY web/nginx/security-headers.conf /etc/nginx/snippets/security-headers.conf
COPY web/nginx/bucket-headers.conf /etc/nginx/snippets/bucket-headers.conf
COPY web/nginx/default.conf.template /etc/nginx/templates/default.conf.template
COPY --from=build /repo/web/dist/r16a-stream-web/browser /usr/share/nginx/html
ENV R16A_WEB_LISTEN=8080 R16A_BACKEND_UPSTREAM=backend:8081 R16A_GARAGE_UPSTREAM=garage:3900
USER 101
EXPOSE 8080
