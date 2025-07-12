FROM rust:1.86.0-bookworm AS build

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() { println!("Placeholder build target")}' > ./src/main.rs
RUN cargo build --release
COPY ./.sqlx ./.sqlx
COPY ./src ./src
RUN touch ./src/main.rs && cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update
RUN apt-get install libssl3

COPY --from=build /target/release/attune-server /usr/local/bin/attune-server

USER 1000:1000

ENV RUST_LOG='attune_server=debug,attune_controlplane=debug'
ENTRYPOINT ["attune-server"]
