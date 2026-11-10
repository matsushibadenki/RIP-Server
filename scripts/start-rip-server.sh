#!/bin/bash
# Run in the foreground; all processing workers share one data root/configuration.
set -euo pipefail
cd "$(dirname "$0")/.."
bin_dir="${JRIP_BIN_DIR:-$PWD/target/debug}"
if [ ! -x "$bin_dir/jrip-runtime" ]; then
  echo 'Run bash scripts/build-rip-server.sh first / 先にbuildスクリプトを実行してください / 请先运行build脚本' >&2
  exit 1
fi
export JRIP_BIN_DIR="$bin_dir"
exec "$bin_dir/jrip-runtime" "$@"
