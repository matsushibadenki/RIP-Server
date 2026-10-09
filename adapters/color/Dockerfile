FROM rust:1.98.0-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY services ./services
RUN cargo build --locked --release -p rip-color

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends python3 && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/rip-color /usr/local/bin/rip-color
COPY adapters/color/run.py /usr/local/lib/jrip/color.py
USER 1000:1000
ENTRYPOINT ["python3", "-I", "/usr/local/lib/jrip/color.py"]
