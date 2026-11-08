# Étape 1 : compilation du binaire en mode release.
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# Caches BuildKit : les dépendances ne sont pas recompilées à chaque build.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/css-proxy /usr/local/bin/css-proxy

# Étape 2 : image d'exécution minimale, sans la chaîne de compilation.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/css-proxy /usr/local/bin/css-proxy
# Config par défaut ; à remplacer avec -v ./config.toml:/app/config.toml.
COPY config.toml /app/config.toml
WORKDIR /app
ENV CONFIG=/app/config.toml
USER nobody
EXPOSE 3000
CMD ["css-proxy"]
