#!/bin/sh
# One-shot, idempotent Garage bootstrap through the v2 admin API:
# single-node layout, the S3 key from .env, the assets bucket.
set -eu

API="${GARAGE_ADMIN_URL:-http://garage:3903}"
AUTH="Authorization: Bearer ${GARAGE_ADMIN_TOKEN}"
BUCKET="${S3_BUCKET:-assets}"

call() { # call <endpoint> [json-body]
  if [ $# -gt 1 ]; then
    curl -fsS -H "$AUTH" -H 'Content-Type: application/json' -d "$2" "$API/v2/$1"
  else
    curl -fsS -H "$AUTH" "$API/v2/$1"
  fi
}

until curl -fsS -o /dev/null -H "$AUTH" "$API/v2/GetClusterStatus"; do sleep 1; done

status=$(call GetClusterStatus)
if echo "$status" | grep -q '"layoutVersion": *0'; then
  node=$(echo "$status" | grep -o '"id": *"[0-9a-f]*"' | head -1 | grep -o '[0-9a-f]\{64\}')
  call UpdateClusterLayout "{\"roles\":[{\"id\":\"$node\",\"zone\":\"dc1\",\"capacity\":${GARAGE_CAPACITY_BYTES:-100000000000},\"tags\":[]}]}" >/dev/null
  call ApplyClusterLayout '{"version":1}' >/dev/null
  echo "layout applied for node $node"
fi

if ! call "GetKeyInfo?id=${S3_ACCESS_KEY_ID}" >/dev/null 2>&1; then
  call ImportKey "{\"accessKeyId\":\"${S3_ACCESS_KEY_ID}\",\"secretAccessKey\":\"${S3_SECRET_ACCESS_KEY}\",\"name\":\"app\"}" >/dev/null
  echo "key imported"
fi

if ! call "GetBucketInfo?globalAlias=${BUCKET}" >/dev/null 2>&1; then
  call CreateBucket "{\"globalAlias\":\"${BUCKET}\"}" >/dev/null
  echo "bucket $BUCKET created"
fi
bucket_id=$(call "GetBucketInfo?globalAlias=${BUCKET}" | grep -o '"id": *"[0-9a-f]*"' | head -1 | grep -o '[0-9a-f]\{64\}')
call AllowBucketKey "{\"bucketId\":\"$bucket_id\",\"accessKeyId\":\"${S3_ACCESS_KEY_ID}\",\"permissions\":{\"read\":true,\"write\":true,\"owner\":true}}" >/dev/null
echo "garage ready"
