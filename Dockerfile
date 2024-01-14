FROM rust:slim-buster as builder

WORKDIR /project

COPY ./src/ ./src/
COPY ./Cargo.toml .

RUN cargo build --release

FROM debian:buster-slim

COPY --from=builder /project/target/release/unique-id-generator /project/unique-id-generator

EXPOSE 8080

CMD ["/project/unique-id-generator"]
