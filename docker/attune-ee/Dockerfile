FROM rust:1.91.0-trixie AS build

RUN apt-get update && apt-get install -y pkg-config libgpg-error-dev libgpgme-dev

COPY Cargo.toml Cargo.lock ./
COPY ./packages/attune/Cargo.toml ./packages/attune/Cargo.toml
RUN mkdir -p packages/attune/src && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune/src/main.rs
COPY ./packages/attune-macros/Cargo.toml ./packages/attune-macros/Cargo.toml
RUN mkdir -p packages/attune-macros/src && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune-macros/src/lib.rs
COPY ./packages/attune-ee/Cargo.toml ./packages/attune-ee/Cargo.toml
RUN mkdir -p packages/attune-ee/controlplane/src/bin/attune-server-ee && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune-ee/controlplane/src/bin/attune-server-ee/main.rs && \
    mkdir -p packages/attune-ee/controlplane/src/bin/attunectl && \
    echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune-ee/controlplane/src/bin/attunectl/main.rs
RUN cargo build --release
# This removes the placeholder `main.rs` files, which otherwise will overlay
# with the real packages (because the placeholders are in default target
# position, and the real packages don't have entrypoints there).
RUN rm -r ./packages
# We need to explicitly clear the cache for the macros package, which otherwise
# will be built incorrectly using the previously cached placeholder `main.rs`
# files.
RUN rm ./target/release/libattune_macros.d ./target/release/libattune_macros.so ./target/release/deps/libattune_macros-*.so ./target/release/deps/attune_macros-*.d
COPY ./.sqlx ./.sqlx
COPY ./packages ./packages
COPY ./scripts ./scripts
COPY ./docker/migrate/prisma/migrations ./docker/migrate/prisma/migrations
RUN cargo build --release

FROM debian:trixie-slim AS runtime

RUN apt-get update
RUN apt-get install -y --no-install-recommends libssl3 ca-certificates libgpg-error0 libgpgme11 curl && rm -rf /var/lib/apt/lists/*

COPY --from=build /target/release/attune-server-ee /usr/local/bin/attune-server-ee

USER 1000:1000

ENV RUST_LOG=attune_server_ee=debug,attune=debug
HEALTHCHECK --interval=1s --timeout=5s CMD curl -f http://localhost:3000/api/v0/health
ENTRYPOINT ["attune-server-ee"]
