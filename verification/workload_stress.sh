#!/usr/bin/env bash
set -euo pipefail

DURATION="${1:-30}"
CPU_COUNT="${2:-1}"

stress-ng --cpu "$CPU_COUNT" --timeout "${DURATION}s" --metrics-brief
