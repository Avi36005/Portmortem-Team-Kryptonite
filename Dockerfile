# One command to a runnable artifact:
#     docker build -t croniter-rs . && docker run --rm croniter-rs next '0 9 * * 1-5' -n 5
#
# Stage 1 builds the port. Stage 2 contains the binary and nothing else --
# no Python, no interpreter, no build toolchain. That the runtime image can
# have zero Python installed and still work is the point: the shipped artifact
# carries no source-language runtime.

FROM rust:1.97-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY pybridge ./pybridge
# Build only the shipped crate. pybridge (PyO3) is deliberately not built here:
# it is test scaffolding and is not part of the artifact.
RUN cargo build --release -p croniter-core

FROM debian:bookworm-slim
COPY --from=build /src/target/release/croniter /usr/local/bin/croniter
ENTRYPOINT ["croniter"]
CMD ["--help"]
