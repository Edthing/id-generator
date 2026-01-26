FROM rust:slim-bookworm AS builder

WORKDIR /project

COPY ./src/ ./src/
COPY ./Cargo.toml .
COPY ./Cargo.lock .

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /project/target/release/id-generator /project/id-generator

EXPOSE 8080


HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["/project/id-generator"]
