# Multi-stage build for QNAP Container Station (x86_64 or arm64)
FROM rust:1-slim AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/nas_indexd /usr/local/bin/nas_indexd
ENV INDEX_ROOT=/data \
    LISTEN_ADDR=0.0.0.0:9876 \
    REFRESH_SECS=0
EXPOSE 9876
ENTRYPOINT ["/usr/local/bin/nas_indexd"]
