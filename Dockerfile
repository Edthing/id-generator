FROM rust:slim-bookworm AS builder

WORKDIR /project

COPY ./src/ ./src/
COPY ./Cargo.toml .
COPY ./Cargo.lock .

RUN cargo build --release

FROM debian:bookworm-slim

COPY --from=builder /project/target/release/unique-id-generator /project/unique-id-generator

EXPOSE 8080

CMD ["/project/unique-id-generator"]
