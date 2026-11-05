# syntax=docker/dockerfile:1
#
# Throwaway image for spike #172. Build from this directory:
#   docker build -t scrobbl-spike:qwen3-1.7b .
#
# The model is never baked into the image. Mount it read-only at runtime:
#   docker run --rm --read-only --tmpfs /tmp \
#     -v /path/to/models:/models:ro -v /path/to/results:/results \
#     scrobbl-spike:qwen3-1.7b --model /models/<model>.gguf --out /results/run.json

FROM rust:1.99.1-bookworm AS builder
SHELL ["/bin/bash", "-c"]

# bindgen (pulled in by llama-cpp-sys-2) needs libclang; llama.cpp needs cmake
# and a C++ toolchain. The NAS build must prove these are enough.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY schema ./schema
COPY corpus ./corpus

# llama-cpp-2 is compiled with default-features = false, i.e. without OpenMP.
# Pass CARGO_FEATURES=openmp to measure the libgomp variant.
ARG CARGO_FEATURES=""
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release \
        ${CARGO_FEATURES:+--features "$CARGO_FEATURES"} \
    && cp target/release/scrobbl-ask-spike /tmp/scrobbl-ask-spike

FROM debian:bookworm-slim AS runtime

# Only the OpenMP variant needs libgomp1. Keep the default image minimal.
ARG INSTALL_LIBGOMP=0
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libstdc++6 \
        ${INSTALL_LIBGOMP:+libgomp1} \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 spike \
    && useradd --uid 10001 --gid 10001 --no-create-home spike \
    && install -d -o 10001 -g 10001 -m 0700 /data

WORKDIR /app
COPY --from=builder /tmp/scrobbl-ask-spike /usr/local/bin/scrobbl-ask-spike
COPY schema ./schema
COPY corpus ./corpus

USER 10001:10001
ENV TMPDIR=/tmp
ENTRYPOINT ["/usr/local/bin/scrobbl-ask-spike"]
