FROM rust:1.88-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release --bin arbiter

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/arbiter /usr/local/bin/arbiter
COPY config/example-config.yaml /app/config/example-config.yaml

EXPOSE 8080
ENTRYPOINT ["arbiter"]
CMD ["serve", "--config", "/app/config/example-config.yaml"]
