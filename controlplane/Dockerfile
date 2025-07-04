FROM rust:1.86.0-bookworm

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() { println!("Placeholder build target")}' > ./src/main.rs
RUN cargo build --release
COPY ./.sqlx ./.sqlx
COPY ./src ./src
RUN touch ./src/main.rs && cargo build --release

ENV RUST_LOG=trace
ENTRYPOINT ["cargo", "run", "--release", "--bin", "attune-server"]
