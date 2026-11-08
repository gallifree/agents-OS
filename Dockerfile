# Multi-arch: docker buildx build --platform linux/amd64,linux/arm64 .
# Each platform builds natively (QEMU or native runners), so no cross-compilation setup.

FROM node:24-slim AS web
WORKDIR /web
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
COPY web/package.json web/pnpm-lock.yaml ./
RUN corepack pnpm install --frozen-lockfile
COPY web/ ./
RUN corepack pnpm build

FROM rust:1.99-slim AS api
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
# shortcut: no dependency layer caching (cargo-chef), add it when CI build times hurt.
RUN cargo build --release --locked -p api

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
COPY --from=api /src/target/release/api /app/api
COPY --from=web /web/dist/client /app/web
ENV BIND_ADDR=0.0.0.0:8080 WEB_DIR=/app/web
EXPOSE 8080
ENTRYPOINT ["/app/api"]
