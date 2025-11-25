# https://towardsdev.com/minio-latest-security-release-now-available-as-alpine-minio-docker-image-440af8e429ae
# https://github.com/minio/minio/issues/21647

FROM alpine/minio:RELEASE.2025-10-15T17-29-55Z

# `mc` is not included in alpine/minio, so we install it manually.
USER root
ARG TARGETARCH
RUN wget -q https://dl.min.io/client/mc/release/linux-${TARGETARCH}/mc -O /usr/local/bin/mc && \
    chmod +x /usr/local/bin/mc
COPY --chmod=755 ./entrypoint.sh /entrypoint.sh
USER minio

ENTRYPOINT [ "/bin/sh", "/entrypoint.sh" ]
