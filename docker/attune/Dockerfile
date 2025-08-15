FROM rust:1.89.0-bookworm AS build

RUN apt-get update && apt-get install -y pkg-config libgpg-error-dev libgpgme-dev

COPY Cargo.toml Cargo.lock ./
COPY ./packages/attune/Cargo.toml ./packages/attune/Cargo.toml
COPY ./packages/attune-macros ./packages/attune-macros
COPY ./docker/migrate/prisma/migrations ./docker/migrate/prisma/migrations
RUN mkdir -p packages/attune/src && echo 'fn main() { println!("Placeholder build target")}' > ./packages/attune/src/main.rs
RUN cargo build --release
RUN rm -f packages/attune/src/main.rs && rm -f packages/attune-macros/src/lib.rs
COPY ./.sqlx ./.sqlx
COPY ./packages ./packages
RUN cargo build --release --bin attune-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update
RUN apt-get install -y --no-install-recommends libssl3 ca-certificates libgpg-error0 libgpgme11 curl && rm -rf /var/lib/apt/lists/*

COPY --from=build /target/release/attune-server /usr/local/bin/attune-server

USER 1000:1000

ENV RUST_LOG=attune_server=debug,attune=debug
HEALTHCHECK --interval=1s --timeout=5s CMD curl -f http://localhost:3000/api/v0/health
ENTRYPOINT ["attune-server"]
