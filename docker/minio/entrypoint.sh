#!/bin/bash

# See: https://github.com/minio/minio/issues/4769
mkdir /data/attune-dev-0
exec minio server /data --console-address ":9001"
