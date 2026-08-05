# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS frontend-builder
WORKDIR /build/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1.96-bookworm AS backend-builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake perl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build/backend
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/assets ./assets
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home documentx \
    && mkdir -p /app/web /data/documentx/knowledge /data/documentx/templates \
    && chown -R documentx:documentx /app /data/documentx

COPY --from=backend-builder /build/backend/target/release/documentx /usr/local/bin/documentx
COPY --from=frontend-builder --chown=documentx:documentx /build/frontend/dist/ /app/web/
COPY --chown=documentx:documentx deploy/AGENTS.md /data/documentx/AGENTS.md

USER documentx
WORKDIR /app

# 无 config.toml 也可运行；所有配置均能由 DOCUMENTX_* 环境变量注入。
ENV DOCUMENTX_SERVER_HOST=0.0.0.0 \
    DOCUMENTX_SERVER_BASE_PATH=/documentx \
    DOCUMENTX_PATHS_STATIC_DIR=/app/web \
    DOCUMENTX_PATHS_KNOWLEDGE_DIR=/data/documentx/knowledge \
    DOCUMENTX_PATHS_TEMPLATES_DIR=/data/documentx/templates \
    DOCUMENTX_PATHS_AGENTS_FILE=/data/documentx/AGENTS.md

EXPOSE 8080
ENTRYPOINT ["documentx"]
