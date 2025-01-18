# syntax=docker/dockerfile:1

FROM rust:1.84-slim-bullseye AS builder
WORKDIR /app

RUN apt-get update -y \
  && apt-get install -y pkg-config make g++ libssl-dev

RUN --mount=type=bind,source=src,target=src \
  --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
  --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
  --mount=type=cache,target=/app/target/ \
  --mount=type=cache,target=/usr/local/cargo/registry/ \
  <<EOF
set -e
cargo build --locked --release
cp ./target/release/sanitarr /app
EOF

FROM debian:bullseye-slim
WORKDIR /app
COPY --from=builder /app/sanitarr .
COPY entrypoint.sh .
RUN chmod +x entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]
