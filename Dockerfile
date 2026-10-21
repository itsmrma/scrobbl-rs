# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim@sha256:91acba91b842bec0bf7ea6f5e09c644c7ccad72c4996ed9616d861d1c7cd60d4 AS frontend
WORKDIR /work
COPY package.json package-lock.json ./
RUN npm ci
COPY frontend ./frontend
COPY vite.config.mts ./
COPY scripts/frontend-notices.cjs ./scripts/frontend-notices.cjs
RUN npm run build

FROM rust:1.99.1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69 AS build
RUN apt-get update && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/* && rustup component add rust-docs
WORKDIR /work
COPY . .
COPY --from=frontend /work/static/dist ./static/dist
COPY --from=frontend /work/node_modules ./node_modules
RUN cargo build --locked --release
# Keep exact dependency source, native licenses and build inputs alongside every binary.
RUN mkdir -p .cargo && cargo vendor --locked vendor > .cargo/config.toml
ARG VCS_REF=unknown
RUN python3 scripts/package-distribution.py --revision "$VCS_REF" --output /distribution

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 scrobbl && useradd --uid 10001 --gid 10001 --no-create-home scrobbl \
    && install -d -o 10001 -g 10001 -m 0700 /data
WORKDIR /app
COPY --from=build /work/target/release/scrobbl-rs /usr/local/bin/scrobbl-rs
COPY --from=build /distribution /usr/share/scrobbl-rs
# Debian preserves installed system-library copyright/license files here.
RUN dpkg-query -W -f='${Package}\t${Version}\n' > /usr/share/scrobbl-rs/system-packages.tsv \
    && mkdir /usr/share/scrobbl-rs/system-licenses \
    && cp -a /usr/share/doc/. /usr/share/scrobbl-rs/system-licenses/
ARG VCS_REF=unknown
ARG VERSION=development
LABEL org.opencontainers.image.source="https://github.com/itsmrma/scrobbl-rs" \
      org.opencontainers.image.revision="$VCS_REF" \
      org.opencontainers.image.version="$VERSION" \
      org.opencontainers.image.licenses="GPL-3.0-or-later" \
      org.opencontainers.image.title="scrobbl.rs"
ENV SCROBBL_BIND=0.0.0.0 SCROBBL_PORT=3210 SCROBBL_DATA_DIR=/data \
    SCROBBL_EXTERNAL_ORIGIN=http://127.0.0.1:3210
USER 10001:10001
EXPOSE 3210
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/scrobbl-rs", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/scrobbl-rs"]
