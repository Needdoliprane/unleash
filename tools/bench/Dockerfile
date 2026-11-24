# Banc de qualification Unleash : toolchain épinglée, dépendances système de la CI,
# Node 22 et pnpm avec magasin préchargé depuis le verrou du renderer.
# Construction : docker build --tag unleash-bench:1.94.1-bookworm-20261124 <contexte>
# Le contexte contient package.json et pnpm-lock.yaml d'apps/gui-renderer.
FROM rust:1.94.1-bookworm

ARG PNPM_VERSION=10.34.9
# Liste identique à .github/workflows/ci.yml (Tauri 2 sous Linux).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
        patchelf pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=node:22-bookworm-slim /usr/local/bin/node /usr/local/bin/node
COPY --from=node:22-bookworm-slim /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && npm install --global --ignore-scripts pnpm@${PNPM_VERSION}

COPY context/package.json context/pnpm-lock.yaml /opt/renderer-seed/
RUN cd /opt/renderer-seed \
    && pnpm install --frozen-lockfile --ignore-scripts --store-dir /opt/pnpm-store \
    && rm -rf /opt/renderer-seed/node_modules

# Composants de rust-toolchain.toml absents de l'image officielle rust.
# `cargo +1.94.1` contourne la surcharge rustup : les installer une fois, hors réseau ensuite.
RUN rustup component add rustfmt clippy rust-src
