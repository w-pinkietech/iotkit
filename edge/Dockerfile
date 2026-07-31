FROM rust:1.95-bookworm AS build

WORKDIR /src
COPY . .
RUN cargo build --locked --release -p iotkit-edge \
    && install -D --mode=0755 target/release/iotkit-edge /out/iotkit-edge

FROM debian:bookworm-slim AS runtime-base
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home iotkit-edge
COPY --from=build /out/iotkit-edge /usr/local/bin/iotkit-edge
COPY --chmod=0755 edge/docker-entrypoint.sh /usr/local/bin/iotkit-edge-entrypoint
USER iotkit-edge
ENTRYPOINT ["/usr/local/bin/iotkit-edge-entrypoint"]

FROM runtime-base AS runtime

FROM rust:1.95-bookworm AS trial-build
WORKDIR /src
RUN apt-get update \
    && apt-get install --yes --no-install-recommends pkg-config libudev-dev \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --locked --release \
    -p iotkit-edge -p iotkit-edge-node -p iotkit-edge-nodectl

FROM debian:bookworm-slim AS trial-runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libudev1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home iotkit
COPY --from=trial-build /src/target/release/iotkit-edge /usr/local/bin/iotkit-edge
COPY --from=trial-build /src/target/release/iotkit-edge-node /usr/local/bin/iotkit-edge-node
COPY --from=trial-build /src/target/release/iotkit-edge-nodectl /usr/local/bin/iotkit-edge-nodectl
USER iotkit

FROM postgres:17-bookworm AS postgres-tools

FROM runtime-base AS runtime-postgres
USER root
RUN apt-get update \
    && apt-get install --yes --no-install-recommends libpq5 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=postgres-tools /usr/lib/postgresql/17/bin/pg_dump /usr/local/bin/pg_dump
COPY --from=postgres-tools /usr/lib/postgresql/17/bin/pg_restore /usr/local/bin/pg_restore
USER iotkit-edge
