# syntax=docker/dockerfile:1
FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.97-slim AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY server/ server/
# Two jobs keeps the build under the CI runner's memory limit.
RUN CARGO_BUILD_JOBS=2 cargo build --release -p gnotes-server

FROM debian:trixie-slim
# CA roots for outgoing HTTPS, e.g. a hosted speech-to-text service.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd --system --home /data gnotes && mkdir -p /data && chown gnotes /data
COPY --from=server /src/target/release/gnotes-server /usr/local/bin/gnotes-server
COPY --from=web /src/web/dist /app/web
ENV GNOTES_DATA_DIR=/data GNOTES_WEB_DIR=/app/web GNOTES_BIND=0.0.0.0:8080
USER gnotes
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["gnotes-server"]
