# Task 151 — Docker image for ferricd (headless daemon)
FROM rust:1.85-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p ferricd

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/ferricd /usr/local/bin/ferricd
ENV RUST_LOG=info
# API still binds 127.0.0.1 inside container; publish via compose host network or sidecar proxy carefully.
EXPOSE 0
VOLUME ["/data"]
ENTRYPOINT ["ferricd"]
