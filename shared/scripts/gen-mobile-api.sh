#!/bin/sh
# Regenerates the mobile API models from docs/openapi.json with swagger_parser (SPEC 12).
set -eu
cd "$(dirname "$0")/../../mobile"
fvm dart run swagger_parser
fvm dart format lib/core/api/generated >/dev/null
