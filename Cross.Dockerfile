FROM ghcr.io/cross-rs/armv7-unknown-linux-musleabihf:latest AS toolchain

FROM ubuntu:24.04

# Copy the entire ARM musl cross-compilation toolchain from the cross image.
# The toolchain was built with --prefix=/usr/local so everything lives there.
COPY --from=toolchain /usr/local /usr/local

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    cmake \
    make \
    pkg-config \
    gcc \
    autoconf \
    automake \
    libtool \
    && rm -rf /var/lib/apt/lists/*

# Install Rust matching the host version
ARG RUST_VERSION=1.93.1
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain ${RUST_VERSION} --profile minimal \
    --target armv7-unknown-linux-musleabihf --no-modify-path

ENV PATH="/root/.cargo/bin:${PATH}"
