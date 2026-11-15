# syntax=docker/dockerfile:1
# Linux validation only: no desktop packaging and no host user configuration.
FROM node:24-bookworm-slim AS node
FROM rust:1.96.1-bookworm

COPY --from=node /usr/local/ /usr/local/
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl fish zsh \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --shell /bin/bash tester \
    && mkdir -p /app /cache/target \
    && chown -R tester:tester /app /cache /home/tester

ENV HOME=/home/tester \
    CARGO_HOME=/home/tester/.cargo \
    CARGO_TARGET_DIR=/cache/target \
    CARGO_BUILD_JOBS=6
WORKDIR /app
USER tester

COPY --chown=tester:tester package.json package-lock.json ./
RUN --mount=type=cache,target=/home/tester/.npm,uid=1000,gid=1000 \
    npm ci --no-audit --no-fund

# Explicit inputs keep host node_modules, target, credentials and .git out.
COPY --chown=tester:tester index.html svelte.config.js tsconfig.json tsconfig.node.json vite.config.ts ./
COPY --chown=tester:tester src ./src
COPY --chown=tester:tester public ./public
RUN npm run check && npm run test:frontend && npm run build:web

COPY --chown=tester:tester Cargo.toml Cargo.lock ./
COPY --chown=tester:tester crates ./crates
COPY --chown=tester:tester src-tauri ./src-tauri
RUN --mount=type=cache,target=/home/tester/.cargo/registry,uid=1000,gid=1000 \
    --mount=type=cache,target=/home/tester/.cargo/git,uid=1000,gid=1000 \
    --mount=type=cache,target=/cache/target,uid=1000,gid=1000 \
    cargo test -p installer-core -p installer-web --locked \
    && cargo build -p installer-web --locked \
    && mkdir -p /app/target/debug \
    && cp /cache/target/debug/installer-web /app/target/debug/installer-web

CMD ["bash"]
