FROM rust:1.86.0-bookworm AS build

COPY Cargo.toml Cargo.lock ./
COPY ./packages/attune/Cargo.toml ./packages/attune/Cargo.toml
RUN mkdir -p packages/attune/src && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune/src/main.rs
RUN cargo build --release
COPY ./.sqlx ./.sqlx
COPY ./packages ./packages
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update
RUN apt-get install -y libssl3 ca-certificates

COPY --from=build /target/release/attune-server /usr/local/bin/attune-server

USER 1000:1000

ENV RUST_LOG=attune_server=debug,attune=debug
ENTRYPOINT ["attune-server"]
