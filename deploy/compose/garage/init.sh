#!/bin/sh
# Idempotent setup of the development Garage, the same buckets, keys and grants as production.
#
# Runs in the curl image because the Garage image has no shell. It talks to the admin API with
# the dev-only admin token, then proves the key scoping over the S3 API and fails loudly if a
# key can write where it must not.
set -eu

ADMIN="${GARAGE_ADMIN_URL:-http://garage:3903}"
S3="${GARAGE_S3_URL:-http://garage:3900}"
TOKEN="${GARAGE_ADMIN_TOKEN:?set GARAGE_ADMIN_TOKEN}"
MEDIA=r16a-stream-media
UPLOADS=r16a-stream-uploads

# Garage pretty-prints JSON, so responses are flattened to one line before sed reads them.
api() {
  method=$1
  path=$2
  shift 2
  curl -sS -f -X "$method" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' "$@" "$ADMIN$path" |
    tr -d ' \n'
}

echo "waiting for Garage"
i=0
until curl -sS -f -o /dev/null -H "Authorization: Bearer $TOKEN" "$ADMIN/v1/status"; do
  i=$((i + 1))
  [ "$i" -gt 60 ] && { echo "Garage did not come up" >&2; exit 1; }
  sleep 1
done

status=$(api GET /v1/status)
node=$(echo "$status" | sed -n 's/.*"node":"\([0-9a-f]*\)".*/\1/p')
[ -n "$node" ] || { echo "could not read the node id" >&2; exit 1; }

# The layout is assigned once. A node without a role has no capacity and refuses writes.
if echo "$status" | grep -q '"layoutVersion":0'; then
  echo "assigning layout to node $node"
  api POST /v1/layout --data "[{\"id\":\"$node\",\"zone\":\"dev\",\"capacity\":10000000000,\"tags\":[]}]" >/dev/null
  version=$(api GET /v1/layout | sed -n 's/.*"version":\([0-9]*\).*/\1/p' | head -n 1)
  api POST /v1/layout/apply --data "{\"version\":$((version + 1))}" >/dev/null
fi

bucket_id() {
  curl -sS -H "Authorization: Bearer $TOKEN" "$ADMIN/v1/bucket?globalAlias=$1" | tr -d ' \n' |
    sed -n 's/^{"id":"\([0-9a-f]*\)".*/\1/p'
}

for b in $MEDIA $UPLOADS; do
  if [ -z "$(bucket_id "$b")" ]; then
    echo "creating bucket $b"
    api POST /v1/bucket --data "{\"globalAlias\":\"$b\"}" >/dev/null
  fi
done
media_id=$(bucket_id $MEDIA)
uploads_id=$(bucket_id $UPLOADS)

import_key() {
  # Garage answers 400 for an unknown key id, so a failed lookup means the key is missing.
  if ! curl -s -f -o /dev/null -H "Authorization: Bearer $TOKEN" "$ADMIN/v1/key?id=$2"; then
    echo "importing key $1"
    api POST /v1/key/import --data "{\"name\":\"$1\",\"accessKeyId\":\"$2\",\"secretAccessKey\":\"$3\"}" >/dev/null
  fi
}

import_key r16a-stream-api "$API_KEY_ID" "$API_SECRET"
import_key r16a-stream-worker "$WORKER_KEY_ID" "$WORKER_SECRET"

allow() {
  api POST /v1/bucket/allow --data \
    "{\"bucketId\":\"$1\",\"accessKeyId\":\"$2\",\"permissions\":{\"read\":$3,\"write\":$4,\"owner\":false}}" >/dev/null
}

# API key reads media and reads and writes uploads. Worker key reads uploads and reads and
# writes media. Grants are only ever added, so remove a stale grant by hand if one appears.
allow "$media_id" "$API_KEY_ID" true false
allow "$uploads_id" "$API_KEY_ID" true true
allow "$uploads_id" "$WORKER_KEY_ID" true false
allow "$media_id" "$WORKER_KEY_ID" true true

put_status() {
  curl -sS -o /dev/null -w '%{http_code}' -X PUT --data-binary 'scope check' \
    --aws-sigv4 "aws:amz:garage:s3" --user "$1:$2" "$S3/$3/_init/scope-check"
}

delete_object() {
  curl -sS -o /dev/null -X DELETE --aws-sigv4 "aws:amz:garage:s3" --user "$1:$2" "$S3/$3/_init/scope-check"
}

expect() {
  if [ "$1" != "$2" ]; then
    echo "SCOPE CHECK FAILED, $3 returned $1, expected $2" >&2
    exit 1
  fi
  echo "scope check ok, $3 returned $1"
}

expect "$(put_status "$API_KEY_ID" "$API_SECRET" $MEDIA)" 403 "api key writing media"
expect "$(put_status "$WORKER_KEY_ID" "$WORKER_SECRET" $UPLOADS)" 403 "worker key writing uploads"
expect "$(put_status "$API_KEY_ID" "$API_SECRET" $UPLOADS)" 200 "api key writing uploads"
expect "$(put_status "$WORKER_KEY_ID" "$WORKER_SECRET" $MEDIA)" 200 "worker key writing media"
delete_object "$API_KEY_ID" "$API_SECRET" $UPLOADS
delete_object "$WORKER_KEY_ID" "$WORKER_SECRET" $MEDIA

echo "garage init done"
