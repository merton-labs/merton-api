# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.99

FROM rust:${RUST_VERSION}-slim-trixie AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && cp target/release/merton-api /usr/local/bin/merton-api

FROM debian:trixie-slim AS runtime

RUN groupadd --system --gid 10001 merton \
    && useradd --system --uid 10001 --gid merton --no-create-home --shell /usr/sbin/nologin merton \
    && mkdir -p /app/data \
    && chown merton:merton /app/data

WORKDIR /app
COPY --from=builder /usr/local/bin/merton-api /usr/local/bin/merton-api

ENV APP_HOST=0.0.0.0 \
    APP_PORT=8080 \
    DATABASE_URL=sqlite:///app/data/merton.db \
    RUST_LOG=merton_api=info,tower_http=info,sqlx=warn

USER merton
EXPOSE 8080
VOLUME ["/app/data"]

ENTRYPOINT ["/usr/local/bin/merton-api"]
