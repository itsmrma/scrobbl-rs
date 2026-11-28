# syntax=docker/dockerfile:1
FROM node:26-bookworm-slim@sha256:5df79989ed685e09f0a98019db4cebfb22fb9df1a53c0161664a011f7ff6b0f1 AS frontend
WORKDIR /work
COPY package.json package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --ignore-scripts
COPY frontend ./frontend
COPY vite.config.mts ./
COPY scripts/lint-frontend.cjs ./scripts/lint-frontend.cjs
COPY scripts/frontend-notices.cjs ./scripts/frontend-notices.cjs
RUN npm run build

FROM rust:1.100.0-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS build
# cmake, clang, libclang-dev and python3 build the CPU-only llama.cpp runtime
# behind the default `llama` feature. No GPU toolchain is installed.
# `llama-cpp-2` uses default-features = false, so its OpenMP feature is off
# (GGML_OPENMP=OFF); the runtime stage therefore does not need libgomp1.
RUN apt-get update && apt-get install -y --no-install-recommends \
        python3 cmake clang libclang-dev \
    && rm -rf /var/lib/apt/lists/* && rustup component add rust-docs
WORKDIR /work
COPY . .
COPY --from=frontend /work/static/dist ./static/dist
COPY --from=frontend /work/node_modules ./node_modules
ARG VCS_REF=unknown
ENV SCROBBL_BUILD_REF=$VCS_REF
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/work/target \
    cargo build --locked --release && \
    mkdir -p /artifacts && \
    cp /work/target/release/scrobbl-rs /artifacts/scrobbl-rs
# Keep exact dependency source, native licenses and build inputs alongside every binary.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    mkdir -p .cargo && cargo vendor --locked vendor > .cargo/config.toml
RUN python3 scripts/package-distribution.py --revision "$VCS_REF" --output /distribution

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS runtime
# libstdc++6 is the C++ runtime linked by llama.cpp. libgomp1 is intentionally
# absent because the runtime is built without OpenMP; no GPU libraries either.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 \
    && rm -rf /var/lib/apt/lists/* /var/cache/apt/* /var/log/* \
    && groupadd --gid 10001 scrobbl && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin scrobbl \
    && install -d -o 10001 -g 10001 -m 0700 /data
WORKDIR /app
COPY --from=build /artifacts/scrobbl-rs /usr/local/bin/scrobbl-rs
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
    SCROBBL_EXTERNAL_ORIGIN=http://127.0.0.1:3210 MALLOC_ARENA_MAX=2
USER 10001:10001
EXPOSE 3210
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/scrobbl-rs", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/scrobbl-rs"]
