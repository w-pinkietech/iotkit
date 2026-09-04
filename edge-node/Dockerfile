# Edge Node image for the trial profile (deploy/compose.trial.yaml). Builds the
# node and the operator CLI from the workspace; the Broker is the pinned
# eclipse-mosquitto image. Field installations run the binaries under systemd.
FROM rust:1.98-bookworm AS build
WORKDIR /src
RUN apt-get update \
    && apt-get install --yes --no-install-recommends pkg-config libudev-dev \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --locked --release -p iotkit-edge-node -p iotkit-edge-nodectl

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libudev1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home iotkit
COPY --from=build /src/target/release/iotkit-edge-node /usr/local/bin/iotkit-edge-node
COPY --from=build /src/target/release/iotkit-edge-nodectl /usr/local/bin/iotkit-edge-nodectl
USER iotkit
ENTRYPOINT ["/usr/local/bin/iotkit-edge-node"]
