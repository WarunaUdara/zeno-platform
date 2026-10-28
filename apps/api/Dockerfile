# syntax=docker/dockerfile:1.7
FROM rust:1.99-slim-bookworm@sha256:86234e857a413004348b471d11a4ebe1aa92d2eae2b6b143cac68fd1ac3c48be AS build

WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY apps/api/Cargo.toml apps/api/Cargo.toml
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    mkdir -p apps/api/src \
    && touch apps/api/src/lib.rs \
    && printf 'fn main() {}\n' > apps/api/src/main.rs \
    && cargo build --locked --release -p zeno-api
RUN rm -rf apps/api/src
COPY apps/api/src apps/api/src
COPY apps/api/migrations apps/api/migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo clean --release -p zeno-api \
    && cargo build --locked --release -p zeno-api

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8 AS runtime

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 zeno \
    && useradd --system --uid 10001 --gid zeno --home-dir /nonexistent --shell /usr/sbin/nologin zeno

COPY --from=build /source/target/release/zeno-api /usr/local/bin/zeno-api
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/health/ready || exit 1
ENTRYPOINT ["/usr/local/bin/zeno-api"]
CMD ["serve"]
