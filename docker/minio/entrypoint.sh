#!/bin/bash

# This creates a default bucket for the Minio instance, which is required by
# Attune.
#
# See: https://github.com/minio/minio/issues/4769
set -euxo pipefail
minio server /data --console-address ":9001" &
MINIO_SERVER_PID=$!
sleep 1
mc alias set attune http://127.0.0.1:9000 attuneminio attuneminio
mc mb --ignore-existing attune/attune-dev-0
# This needs to be readable for the E2E test to install packages, and writeable
# for integration tests to upload objects.
mc anonymous set public attune/attune-dev-0

# We `exec` instead of just `wait`ing on the PID so that signals propagate
# correctly.
kill $MINIO_SERVER_PID
exec minio server /data --console-address ":9001"
