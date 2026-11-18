#!/usr/bin/env bash
set -euo pipefail
KIT=$(cd "$(dirname "$0")" && pwd)
exec "$KIT/run.sh" all "$@"
