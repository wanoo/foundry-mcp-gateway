# Build multi-étages : image finale minimale, binaire lié à rustls (aucune
# dépendance OpenSSL système).
FROM rust:1-slim AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# Les caches BuildKit évitent de recompiler tout l'arbre de dépendances à chaque
# changement de src/. Le `cp` est OBLIGATOIRE : un /app/target monté en cache
# n'existe plus à l'étape suivante, donc COPY --from ne le verrait pas.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release && \
    cp /app/target/release/foundry-mcp /usr/local/bin/foundry-mcp

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/foundry-mcp /usr/local/bin/foundry-mcp
ENV PORT=8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD curl -fsS "http://localhost:${PORT}/health" || exit 1
CMD ["foundry-mcp"]
