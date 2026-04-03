#!/usr/bin/env bash
# Verify TempoOracle top-of-block tx ordering via tempo-xtask (see scripts/oracle-localnet/README.txt).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
RPC="${RPC_URL:-http://127.0.0.1:8545}"
FROM="${1:-1}"
TO="${2:-}"
if [[ -n "$TO" ]]; then
  exec cargo run -p tempo-xtask -- verify-oracle-blocks --rpc "$RPC" --from-block "$FROM" --to-block "$TO"
else
  exec cargo run -p tempo-xtask -- verify-oracle-blocks --rpc "$RPC" --from-block "$FROM"
fi
