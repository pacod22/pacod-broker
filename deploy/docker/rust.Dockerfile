# syntax=docker/dockerfile:1
#
# The broker's image. Build from the repository root:
#
#   docker build -f deploy/docker/rust.Dockerfile -t pacod-broker .
#
# The build context is narrowed by rust.Dockerfile.dockerignore (next to this file) to the
# Cargo workspace. As pacod-gw's: cargo-chef keeps third-party crates in their own layer, the
# binary is linked statically against musl (mimalloc replaces musl's allocator) and runs on a
# distroless base without a shell or package manager, as user 65532.

ARG RUST_VERSION=1.99.0
ARG CARGO_CHEF_VERSION=0.1.78
# The tag "nonroot" moves; the digest pins it (the one pacod-gw pins). Refresh with:
#   docker buildx imagetools inspect gcr.io/distroless/static-debian13:nonroot
ARG RUNTIME_IMAGE=gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3

FROM lukemathwalker/cargo-chef:${CARGO_CHEF_VERSION}-rust-${RUST_VERSION}-slim-trixie AS chef
RUN apt-get update \
 && apt-get install -y --no-install-recommends musl-tools \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    triple="$(uname -m)-unknown-linux-musl"; \
    rustup target add "$triple"; \
    printf '%s' "$triple" > /rust-target; \
    printf '[build]\ntarget = "%s"\n' "$triple" >> "$CARGO_HOME/config.toml"
ENV CARGO_INCREMENTAL=0 CARGO_TERM_COLOR=never CARGO_NET_RETRY=5
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path /recipe.json

FROM chef AS deps
COPY --from=planner /recipe.json /recipe.json
# The recipe leaves out path dependencies outside the workspace: pacod-id's vendored crates.
COPY vendor/pacod-id/crates /src/vendor/pacod-id/crates
RUN --mount=type=cache,id=pacod-broker-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    cargo chef cook --release --locked --workspace --recipe-path /recipe.json

FROM deps AS build
COPY . .
RUN --mount=type=cache,id=pacod-broker-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    set -eu; \
    cargo build --release --locked --workspace --bin pacod-broker; \
    install -D "target/$(cat /rust-target)/release/pacod-broker" /out/pacod-broker; \
    mkdir -p /out/state

# The health probe: standard library only, for an image without curl.
FROM chef AS healthcheck
COPY deploy/docker/healthcheck.rs /src/healthcheck.rs
RUN rustc --edition 2024 -C opt-level=s -C strip=symbols -C target-feature=+crt-static \
      --target "$(cat /rust-target)" -o /out/healthcheck /src/healthcheck.rs

FROM ${RUNTIME_IMAGE} AS broker
ARG VERSION=dev
ARG VCS_REF=unknown
LABEL org.opencontainers.image.title="pacod-broker" \
      org.opencontainers.image.description="Discovery of AI and compute resources, with prices and availability over time and reputation" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${VCS_REF}"
COPY --from=healthcheck /out/healthcheck /usr/local/bin/healthcheck
COPY --from=build /out/pacod-broker /usr/local/bin/pacod-broker
COPY --from=build --chown=65532:65532 /out/state /var/lib/pacod-broker
USER 65532:65532
EXPOSE 8060 8061 9160
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/usr/local/bin/healthcheck", "http://127.0.0.1:8060/healthz"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/pacod-broker"]
CMD ["serve"]
