FROM golang:1.25-bookworm AS build

WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -o /out/iotkit-edge ./cmd/iotkit-edge

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home iotkit-edge
COPY --from=build /out/iotkit-edge /usr/local/bin/iotkit-edge
USER iotkit-edge
ENTRYPOINT ["/usr/local/bin/iotkit-edge"]
