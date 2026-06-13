# Multi-stage build: compile the CLI, then ship it on a slim runtime.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin mainstage --bin mainstage-lsp

FROM debian:bookworm-slim
# `git` backs the `git` module; `ca-certificates` lets the `http` module use TLS.
RUN apt-get update \
	&& apt-get install -y --no-install-recommends git ca-certificates \
	&& rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/mainstage /usr/local/bin/mainstage
COPY --from=build /src/target/release/mainstage-lsp /usr/local/bin/mainstage-lsp

# Run pipelines against a project mounted at /work.
WORKDIR /work
ENTRYPOINT ["mainstage"]
