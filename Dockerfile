# ── build the console ────────────────────────────────────────────────────
FROM node:22-slim AS console
WORKDIR /build
COPY console/package.json console/package-lock.json* ./
RUN npm install --no-audit --no-fund
COPY console/ ./
RUN npm run build

# ── build the server ─────────────────────────────────────────────────────
FROM rust:1-slim AS server
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p cortex-server

# ── runtime: rust binary + python + node workers ─────────────────────────
FROM node:22-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends python3 ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /build/target/release/cortex-server /usr/local/bin/cortex-server
COPY --from=console /build/dist /app/console/dist
ENV CORTEX_PORT=7420 \
    CORTEX_DATA_DIR=/data \
    CORTEX_CONSOLE_DIST=/app/console/dist
VOLUME /data
EXPOSE 7420
CMD ["cortex-server"]
