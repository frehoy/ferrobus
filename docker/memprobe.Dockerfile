# Runs `ferrobus-core/examples/memprobe` under Linux/glibc.
#
# Build from the repo root:
#   docker build -f docker/memprobe.Dockerfile -t ferrobus-memprobe .
# or use scripts/memprobe-linux.sh, which does that and the mounts.
FROM rust:1.98-slim-trixie AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# Only ferrobus_core: the root package is a pyo3 module and needs a Python build.
RUN cargo build --release --example memprobe -p ferrobus_core

FROM debian:trixie-slim
COPY --from=build /src/target/release/examples/memprobe /usr/local/bin/memprobe
ENTRYPOINT ["memprobe"]
