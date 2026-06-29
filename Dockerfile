# syntax=docker/dockerfile:1.7

FROM node:24-bookworm-slim AS web-build
WORKDIR /app/web

COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm \
    npm ci --no-audit --no-fund

COPY web/ ./
RUN npm run build

FROM rust:1-bookworm AS server-build
WORKDIR /app

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --bin emby-manager && \
    mkdir -p /out && \
    cp /app/target/release/emby-manager /out/emby-manager

FROM debian:bookworm-slim AS runtime

ARG EMBY_MANAGER_UID=10001
ARG EMBY_MANAGER_GID=10001

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    mkdir -p /app/web /legacy /data /media /strm && \
    if ! getent group "${EMBY_MANAGER_GID}" >/dev/null; then groupadd --gid "${EMBY_MANAGER_GID}" emby-manager; fi && \
    if ! getent passwd "${EMBY_MANAGER_UID}" >/dev/null; then useradd --uid "${EMBY_MANAGER_UID}" --gid "${EMBY_MANAGER_GID}" --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin emby-manager; fi && \
    chown -R "${EMBY_MANAGER_UID}:${EMBY_MANAGER_GID}" /app /data

COPY --from=server-build /out/emby-manager /usr/local/bin/emby-manager
COPY --from=web-build /app/web/dist /app/web
RUN chown -R "${EMBY_MANAGER_UID}:${EMBY_MANAGER_GID}" /app/web

ENV EMBY_MANAGER_HOST=0.0.0.0 \
    EMBY_MANAGER_PORT=8098 \
    EMBY_MANAGER_WEB_DIST=/app/web \
    EMBY_MANAGER_LEGACY_DIR=/legacy \
    EMBY_MANAGER_CD_ROOT=/media \
    EMBY_MANAGER_STRM_ROOT=/strm

EXPOSE 8098
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8098/health >/dev/null || exit 1

USER ${EMBY_MANAGER_UID}:${EMBY_MANAGER_GID}
ENTRYPOINT ["emby-manager"]
CMD ["serve"]
