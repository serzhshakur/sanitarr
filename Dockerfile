# syntax=docker/dockerfile:1

FROM rust:1.88-slim-bookworm AS builder
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
mv ./target/release/sanitarr /app
EOF

FROM debian:bookworm-slim AS runtime
RUN apt-get update && \
    apt-get install -y libssl3 && \
    rm -rf /var/cache/apt/archives /var/lib/apt/lists/*
COPY --from=builder /app/sanitarr /usr/local/bin
COPY entrypoint.sh .

ENTRYPOINT ["./entrypoint.sh"]
