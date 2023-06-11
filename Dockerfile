########################################
#          Base Builder Image          #
########################################
FROM docker.io/lukemathwalker/cargo-chef:latest-rust-alpine as chef-factory

WORKDIR /src

RUN set -eux; apk add --no-cache bash musl-dev


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
FROM docker.io/alpine:latest AS app-image

# ^ we don't need the Rust toolchain to run the binary

COPY --from=builder /workspace/target/release/echo-rs /usr/local/bin/echo-rs

WORKDIR /

RUN apk add --no-cache bash less

SHELL ["/bin/bash", "-o", "errexit", "-o", "pipefail", "-o", "nounset", "-c"]

USER 1000

EXPOSE 8080 9090

ENTRYPOINT ["/usr/local/bin/echo-rs"]
