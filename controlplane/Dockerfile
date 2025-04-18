FROM rust:1.86.0-bookworm

ADD . .
RUN cargo build --release

ENV RUST_LOG trace
ENTRYPOINT ["cargo", "run", "--release"]
