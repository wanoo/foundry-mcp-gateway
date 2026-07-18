# Build multi-étages : image finale minimale (~90 Mo), binaire statiquement lié
# à rustls (aucune dépendance OpenSSL système).
FROM rust:1-slim AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/foundry-mcp /usr/local/bin/foundry-mcp
ENV PORT=8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD curl -fsS "http://localhost:${PORT}/health" || exit 1
CMD ["foundry-mcp"]
