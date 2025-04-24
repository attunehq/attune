#!/bin/bash

# This creates a default bucket for the Minio instance, which is required by
# Attune.
#
# See: https://github.com/minio/minio/issues/4769
mkdir /data/attune-dev-0
exec minio server /data --console-address ":9001"
