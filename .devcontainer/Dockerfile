FROM rust:latest

# Avoid interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Core build tools (not included in rust:latest slim base)
    cmake \
    make \
    pkg-config \
    # DBus (required by bluer / dbus crate)
    libdbus-1-dev \
    dbus \
    # SDL2 (bundled build still needs a C compiler and headers)
    libsdl2-dev \
    # clang / llvm (required by bluer's bindgen step)
    clang \
    libclang-dev \
    llvm \
    # cross / Docker-in-Docker dependencies
    docker.io \
    && rm -rf /var/lib/apt/lists/*

# Add cross-compilation target and useful components
RUN rustup component add rust-src rust-analyzer rustfmt clippy \
    && rustup target add armv7-unknown-linux-musleabihf

# Install cross (for armv7 cross-compilation)
RUN cargo install cross --git https://github.com/cross-rs/cross

# Make /root and the cargo/rustup dirs world-writable so that Zed (which runs
# commands as the remapped UID 1000 via --userns=keep-id) can write into them.
RUN chmod 777 /root \
    && chmod -R 777 /usr/local/cargo \
    && chmod -R 777 /usr/local/rustup

WORKDIR /workspace
