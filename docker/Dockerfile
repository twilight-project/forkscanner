FROM rust:latest as build

RUN apt-get update && apt-get install -y gcc \
    libpq-dev \
	g++

COPY . .

RUN cargo b --release

FROM debian:bullseye

RUN apt-get update && apt-get install -y libpq5

ENV RUST_LOG=info
ENV LOG_DIR=/log

WORKDIR /app

COPY .compose.env .env
COPY --from=build target/release/forkscanner .


CMD ["/app/forkscanner", "-a", "all"]
