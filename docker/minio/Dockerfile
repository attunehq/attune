# Building MinIO from source because they no longer provide a prebuilt image:
# https://github.com/minio/minio/issues/21647

# Update this when building new images.
ARG MINIO_RELEASE=RELEASE.2025-10-15T17-29-55Z

FROM golang:1.24-alpine AS minio-builder
ARG MINIO_RELEASE
RUN apk add --no-cache git
RUN git clone --depth 1 --branch ${MINIO_RELEASE} https://github.com/minio/minio.git /src
WORKDIR /src
RUN go build -o /usr/local/bin/minio .

FROM alpine:3.21
COPY --from=minio-builder /usr/local/bin/minio /usr/local/bin/minio

# We can use the prebuilt `mc` binary from MinIO's release page, they still
# publish that.
ARG TARGETARCH
RUN wget -q https://dl.min.io/client/mc/release/linux-${TARGETARCH}/mc -O /usr/local/bin/mc && \
    chmod +x /usr/local/bin/mc
COPY --chmod=755 ./entrypoint.sh /entrypoint.sh

ENTRYPOINT [ "/bin/sh", "/entrypoint.sh" ]
