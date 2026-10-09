#!/bin/bash
# Start only the file-output proof printer, never the physical delivery worker.
set -eu
cd "$(dirname "$0")/.."
: "${JRIP_API_TOKEN:?Set JRIP_API_TOKEN (24+ characters)}"
if [ "${#JRIP_API_TOKEN}" -lt 24 ]; then echo 'JRIP_API_TOKEN must have 24+ characters' >&2; exit 1; fi
export JRIP_DATA_ROOT="${JRIP_DATA_ROOT:-./data}"
mkdir -p "$JRIP_DATA_ROOT/logs"
docker info >/dev/null
docker image inspect "${JRIP_MUPDF_IMAGE:-jrip-mupdf:local}" >/dev/null
cargo build --locked -p jrip-server --bins -p jrip-ipp --bin jrip-ipp
pids=()
cleanup() { trap - EXIT INT TERM; for pid in "${pids[@]}"; do kill -INT "$pid" 2>/dev/null || true; done; for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM
./target/debug/jrip-server >"$JRIP_DATA_ROOT/logs/api.log" 2>&1 & pids+=("$!")
./target/debug/jrip-ipp >"$JRIP_DATA_ROOT/logs/ipp.log" 2>&1 & pids+=("$!")
./target/debug/jrip-dispatcher >"$JRIP_DATA_ROOT/logs/dispatcher.log" 2>&1 & pids+=("$!")
echo 'J-RIP proof printer processes started; TIFF output only. Logs: JRIP_DATA_ROOT/logs'
while true; do
    for pid in "${pids[@]}"; do
        if ! kill -0 "$pid" 2>/dev/null; then echo 'A proof printer process exited; check logs.' >&2; exit 1; fi
    done
    sleep 2
done
