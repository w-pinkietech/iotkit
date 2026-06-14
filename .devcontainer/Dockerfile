FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        i2c-tools \
        libudev-dev \
        mosquitto-clients \
        pkg-config \
        udev \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy
