# syntax=docker/dockerfile:1.7
#
# Container for the public deployment at minmax.watch.
#
# The build stage reproduces the `build-web` recipe in the justfile. Those two
# have to stay in step: every root-path asset the recipe copies is copied here
# too, and anything added there has to be added here or it works under
# `just serve` and 404s in production.
#
# Two things this does that the recipe does not: it brotli-compresses the
# bundle (the server serves the `.br` siblings when the client accepts them),
# and it builds only `overwatch-server` rather than the workspace, leaving out
# the dev-only `overwatch-ingest` and its scraping and image-processing
# dependencies.

# Trixie rather than bookworm, and both stages have to agree on it: the
# prebuilt `dx` release links against glibc 2.39, which bookworm (2.36) cannot
# load. Building the server here and running it on an older base would fail the
# same way, one layer later.
FROM rust:1.96-trixie AS builder

# brotli is a build-time tool here, not a runtime one: the bundle is compressed
# once at -q 11 and the server does plain file reads from then on.
RUN apt-get update \
 && apt-get install -y --no-install-recommends brotli \
 && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown

# Pinned to the `dioxus` dependency in overwatch-web/Cargo.toml — a CLI on a
# disagreeing minor version is not expected to work, so the two move together.
# binstall pulls the prebuilt release; `cargo install` would build the CLI from
# source and cost several minutes on every cold build.
ARG DIOXUS_CLI_VERSION=0.7.10
RUN curl -L --proto '=https' --tlsv1.2 -sSf \
      https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
      | bash \
 && cargo binstall -y --locked "dioxus-cli@${DIOXUS_CLI_VERSION}"

WORKDIR /src

# The whole workspace: overwatch-data reaches the root `data/*.toml` through
# include_str!, and dx writes into the root-level `target/` even though it runs
# from overwatch-web/.
COPY . .

# The build lives in docker/build.sh rather than a chain of shell in the RUN.
# It is long enough to want comments, and a comment inside a line-continued RUN
# depends on how the frontend strips it — a poor bet for a step whose failure
# mode is a silently mangled command.
#
# Both builds run in one layer so they share a warm target directory. The cache
# mounts are what make a rebuild cheap, and they are also why the script copies
# its results into /out: a cache mount is scratch space, not part of the
# resulting image layer, so anything left under target/ would simply not exist
# in the next stage.

# The commit being built, stamped into the bundle's footer and into /health.
# It has to come in as an ARG: .dockerignore excludes .git from the context, so
# nothing inside this stage can work the sha out for itself. CI passes it; a
# local `docker build` without it produces an image that honestly says "dev".
# Declared here rather than at the top of the stage so that a new sha only
# invalidates this layer and not the toolchain ones above it.
ARG MINMAX_BUILD=dev

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    bash docker/build.sh


FROM debian:trixie-slim AS runtime

# Nothing is installed: the binary is an ordinary glibc dynamic link and uses
# rustls rather than OpenSSL, and it never makes an outbound TLS connection, so
# it needs neither libssl nor a CA bundle.
RUN useradd --system --uid 10001 --user-group --no-create-home minmax

COPY --from=builder /out/bin/overwatch-server /usr/local/bin/overwatch-server
COPY --from=builder /out/public /srv/public

# Absolute paths: the defaults for both of these are relative to the working
# directory. OVERWATCH_MATCH_LOG is deliberately empty rather than unset —
# empty switches the match log off and makes /api/matches a 404, which is what
# makes this safe to expose. Unset would fall back to the local default and
# open the endpoint. See the "On exposing it" note in README.md.
ENV OVERWATCH_ADDR=0.0.0.0:8080 \
    OVERWATCH_ASSETS=/srv/public \
    OVERWATCH_MATCH_LOG=""

USER 10001:10001
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/overwatch-server"]
