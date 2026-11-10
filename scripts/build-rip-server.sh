#!/bin/bash
# Build the complete local runtime and its three fixed sandbox images.
set -euo pipefail
cd "$(dirname "$0")/.."
docker info >/dev/null
toolchain="$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)"
: "${toolchain:?Missing pinned Rust toolchain}"
rustup run "$toolchain" cargo build --workspace --bins --locked
docker build -f adapters/ghostscript/Dockerfile -t jrip-ghostscript:local .
docker build -f adapters/mupdf/Dockerfile -t jrip-mupdf:local .
docker build -f adapters/color/Dockerfile -t jrip-color:local .
echo 'Build complete / ビルド完了 / 构建完成'
