########################################
#          Base Builder Image          #
########################################
FROM docker.io/lukemathwalker/cargo-chef:latest-rust-alpine as chef-factory

WORKDIR WORKDIR /workspace

RUN set -eux; apk add --no-cache bash musl-dev \
    && wget -O "/usr/bin/tini" \
    "https://github.com/krallin/tini/releases/download/v0.19.0/tini-static-$(uname -m | sed 's#x86_64#amd64#g; s#aarch64#arm64#g')" \
    && chmod +x /usr/bin/tini


########################################
#            Build "Planner"           #
########################################
FROM chef-factory AS planner

WORKDIR /workspace

COPY . .

SHELL ["/bin/bash", "-o", "errexit", "-o", "pipefail", "-o", "nounset", "-c"]

RUN cargo chef prepare --recipe-path recipe.json


########################################
#           Artifact Builder           #
########################################
FROM chef-factory AS builder

WORKDIR /workspace

COPY --from=planner /workspace/recipe.json recipe.json

SHELL ["/bin/bash", "-o", "errexit", "-o", "pipefail", "-o", "nounset", "-c"]

# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json

# Build application
COPY . .

RUN cargo build --release --bin echo-rs



########################################
#              App Image               #
########################################
FROM gcr.io/distroless/static-debian11:nonroot AS app-image

# ^ we don't need the Rust toolchain to run the binary

WORKDIR /

COPY --from=builder /usr/bin/tini /bin/tini

COPY --from=builder /workspace/target/release/echo-rs /bin/echo-rs

USER 1000:1000

EXPOSE 8080 9090

ENTRYPOINT ["/bin/tini", "--"]

CMD ["/bin/echo-rs"]
