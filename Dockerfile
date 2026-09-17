# ── build the console ────────────────────────────────────────────────────
FROM node:22-bookworm-slim AS console
WORKDIR /build
COPY console/package.json console/package-lock.json* ./
RUN npm install --no-audit --no-fund
COPY console/ ./
RUN npm run build

# ── build the server ─────────────────────────────────────────────────────
# Pinned to bookworm to match the runtime (glibc 2.36). Floating `*-slim`
# tags drifted: rust:1-slim moved to trixie (glibc 2.41) while
# node:22-slim is still bookworm (2.36), producing
# `GLIBC_2.39 not found` at startup.
FROM rust:1-bookworm AS server
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p loom-server

# ── runtime: rust binary + python + node workers ─────────────────────────
FROM node:22-bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends python3 ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /build/target/release/loom-server /usr/local/bin/loom-server
COPY --from=console /build/dist /app/console/dist
ENV LOOM_PORT=7420 \
    LOOM_DATA_DIR=/data \
    LOOM_CONSOLE_DIST=/app/console/dist
VOLUME /data
EXPOSE 7420
CMD ["loom-server"]
