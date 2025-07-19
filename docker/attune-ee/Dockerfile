FROM rust:1.86.0-bookworm AS build

COPY Cargo.toml Cargo.lock ./
COPY ./packages/attune/Cargo.toml ./packages/attune/Cargo.toml
RUN mkdir -p packages/attune/src && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune/src/main.rs
COPY ./packages/attune-ee/Cargo.toml ./packages/attune-ee/Cargo.toml
RUN mkdir -p packages/attune-ee/controlplane/src/bin/attune-server-ee && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune-ee/controlplane/src/bin/attune-server-ee/main.rs && \
    mkdir -p packages/attune-ee/controlplane/src/bin/attunectl && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune-ee/controlplane/src/bin/attunectl/main.rs
RUN cargo build --release
COPY ./.sqlx ./.sqlx
COPY ./packages ./packages
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update
RUN apt-get install libssl3

COPY --from=build /target/release/attune-server-ee /usr/local/bin/attune-server-ee

USER 1000:1000

ENV RUST_LOG=attune_server_ee=debug,attune=debug
ENTRYPOINT ["attune-server-ee"]
