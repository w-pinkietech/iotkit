FROM golang:1.25-bookworm AS build

WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -o /out/iotkit-edge ./cmd/iotkit-edge

FROM debian:bookworm-slim AS runtime-base
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home iotkit-edge
COPY --from=build /out/iotkit-edge /usr/local/bin/iotkit-edge
USER iotkit-edge
ENTRYPOINT ["/usr/local/bin/iotkit-edge"]

FROM runtime-base AS runtime

FROM postgres:17-bookworm AS postgres-tools

FROM runtime-base AS runtime-postgres
USER root
RUN apt-get update \
    && apt-get install --yes --no-install-recommends libpq5 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=postgres-tools /usr/lib/postgresql/17/bin/pg_dump /usr/local/bin/pg_dump
COPY --from=postgres-tools /usr/lib/postgresql/17/bin/pg_restore /usr/local/bin/pg_restore
USER iotkit-edge
