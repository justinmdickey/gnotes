# syntax=docker/dockerfile:1
FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.97-slim AS server
WORKDIR /src
# Dependencies first, against stub sources, so this layer is reused until Cargo.toml or Cargo.lock
# changes and a release only compiles gnotes itself. Two jobs by default keeps it under the homelab
# runner's memory limit; bigger builders pass --build-arg JOBS=4.
ARG JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml server/
RUN mkdir server/src && echo "fn main() {}" > server/src/main.rs && touch server/src/lib.rs \
    && CARGO_BUILD_JOBS=$JOBS cargo build --release -p gnotes-server \
    && rm -rf server/src target/release/.fingerprint/gnotes-server-* target/release/deps/*gnotes_server*
COPY server/ server/
# The release tag (e.g. v0.7.0), shown in Settings. After the dependency layer so it doesn't bust it.
ARG VERSION=dev
RUN GNOTES_VERSION="${VERSION#v}" CARGO_BUILD_JOBS=$JOBS cargo build --release -p gnotes-server

FROM debian:trixie-slim
# CA roots for outgoing HTTPS, e.g. a hosted speech-to-text service.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
# ffmpeg turns recordings into WAV for speech-to-text servers that only read WAV. A static build, since
# Debian's package pulls in a few hundred MB of desktop libraries.
COPY --from=mwader/static-ffmpeg:7.1 /ffmpeg /usr/local/bin/ffmpeg
RUN useradd --system --home /data gnotes && mkdir -p /data && chown gnotes /data
COPY --from=server /src/target/release/gnotes-server /usr/local/bin/gnotes-server
COPY --from=web /src/web/dist /app/web
ENV GNOTES_DATA_DIR=/data GNOTES_WEB_DIR=/app/web GNOTES_BIND=0.0.0.0:8080
USER gnotes
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["gnotes-server"]
