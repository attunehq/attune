#!/bin/sh

# This creates a default bucket for the Minio instance, which is required by
# Attune.
#
# See: https://github.com/minio/minio/issues/4769
set -eux
minio server /data --console-address ":9001" &
MINIO_SERVER_PID=$!

# Wait for minio to be ready (CI can be slow)
MINIO_READY=false
for i in 1 2 3 4 5; do
    if mc alias set local http://127.0.0.1:9000 attuneminio attuneminio 2>/dev/null; then
        MINIO_READY=true
        break
    fi
    echo "Waiting for minio to start (attempt $i)..."
    sleep $i
done
if [ "$MINIO_READY" = "false" ]; then
    echo "ERROR: MinIO failed to start after 5 attempts"
    exit 1
fi

# 'local' alias was set in the loop above. Now set 'attune' alias.
mc alias set attune http://127.0.0.1:9000 attuneminio attuneminio
mc mb --ignore-existing attune/attune-dev-0

# This needs to be readable for the E2E test to install packages, and writeable
# for integration tests to upload objects.
mc anonymous set public attune/attune-dev-0

# We `exec` instead of just `wait`ing on the PID so that signals propagate
# correctly.
kill $MINIO_SERVER_PID
exec minio server /data --console-address ":9001"
