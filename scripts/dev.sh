#!/usr/bin/env bash
# Run the Cortex server and the console dev server side by side.
#   ./scripts/dev.sh
# Server: http://localhost:7420   Console (hot reload): http://localhost:3001
set -euo pipefail
cd "$(dirname "$0")/.."

if [ ! -d console/node_modules ]; then
  (cd console && npm install)
fi

cargo build -p cortex-server

trap 'kill 0' EXIT
cargo run -p cortex-server &
(cd console && npm run dev) &
wait
