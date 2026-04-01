#!/usr/bin/env bash
#
# Raw RAPL baseline measurement for energy verification.
#
# Methodology (mirrors EMT):
#   attributed_energy = total_rapl_energy × (process_cpu_util / system_cpu_util)
#
# We read RAPL zone counters and /proc/stat before & after the workload.
# Process CPU is captured by polling /proc/<pid>/stat while the workload runs;
# the last successful read gives cumulative utime+stime for the process tree.
#
# Usage: sudo bash rapl_baseline.sh <duration_s> [workload_script] [python]
# Output: single JSON object on stdout.

set -euo pipefail

DURATION="${1:-10}"
WORKLOAD="${2:-$(dirname "$0")/workload.py}"
PYTHON="${3:-python3}"

# ── helpers ──────────────────────────────────────────────────────────────────

# Sum energy_uj across RAPL package-level counters only (intel-rapl:N, not intel-rapl:N:M).
# Package energy already includes core + uncore, so we avoid double-counting.
read_rapl_uj() {
    local total=0
    for f in /sys/class/powercap/intel-rapl:*/energy_uj; do
        # Only count top-level packages (e.g., intel-rapl:0, intel-rapl:1)
        # Skip subdomains (e.g., intel-rapl:0:0, intel-rapl:0:1)
        dir=$(dirname "$f")
        name=$(basename "$dir")
        # Count colons - top-level has exactly 1 colon
        colons=$(echo "$name" | tr -cd ':' | wc -c)
        if [ "$colons" -eq 1 ]; then
            if [ -r "$f" ]; then
                total=$(( total + $(cat "$f") ))
            fi
        fi
    done
    echo "$total"
}

# Total CPU jiffies (all cores combined) from /proc/stat.
read_cpu_total() {
    awk '/^cpu / {print $2+$3+$4+$5+$6+$7+$8+$9+$10+$11}' /proc/stat
}

# Cumulative utime+stime for a single PID.
read_proc_cpu() {
    local pid=$1
    # Race-safe read: process can exit between -f and awk.
    if [ -f "/proc/$pid/stat" ]; then
        awk '{print $14+$15}' "/proc/$pid/stat" 2>/dev/null || echo 0
    else
        echo 0
    fi
}

# Recursively sum utime+stime for a PID and all its descendants.
read_tree_cpu() {
    local pid=$1
    local total
    total=$(read_proc_cpu "$pid")
    local children
    children=$(pgrep -P "$pid" 2>/dev/null || true)
    for child in $children; do
        total=$(( total + $(read_tree_cpu "$child") ))
    done
    echo "$total"
}

# ── measurement ──────────────────────────────────────────────────────────────

# Snapshot RAPL + system CPU before workload.
RAPL_BEFORE=$(read_rapl_uj)
CPU_TOTAL_BEFORE=$(read_cpu_total)

# Launch workload in background.
$PYTHON "$WORKLOAD" "$DURATION" > /dev/null 2>&1 &
WL_PID=$!
sleep 0.2

# Poll process-tree CPU while workload is alive.
# We keep the last reading — it represents cumulative jiffies up to that point.
LAST_TREE_CPU=0
POLL_COUNT=0
while kill -0 "$WL_PID" 2>/dev/null; do
    LAST_TREE_CPU=$(read_tree_cpu "$WL_PID")
    POLL_COUNT=$(( POLL_COUNT + 1 ))
    sleep 0.2
done
wait "$WL_PID" 2>/dev/null || true

# Snapshot RAPL + system CPU after workload.
RAPL_AFTER=$(read_rapl_uj)
CPU_TOTAL_AFTER=$(read_cpu_total)

# ── compute ──────────────────────────────────────────────────────────────────

RAPL_DELTA_UJ=$(( RAPL_AFTER - RAPL_BEFORE ))

# Handle counter overflow.
if [ "$RAPL_DELTA_UJ" -lt 0 ]; then
    MAX_UJ=$(cat /sys/class/powercap/intel-rapl:0/max_energy_range_uj 2>/dev/null || echo 0)
    [ "$MAX_UJ" -gt 0 ] && RAPL_DELTA_UJ=$(( RAPL_DELTA_UJ + MAX_UJ ))
fi

CPU_TOTAL_DELTA=$(( CPU_TOTAL_AFTER - CPU_TOTAL_BEFORE ))

RAPL_TOTAL_J=$(awk "BEGIN {printf \"%.4f\", $RAPL_DELTA_UJ / 1000000.0}")

if [ "$CPU_TOTAL_DELTA" -gt 0 ] && [ "$LAST_TREE_CPU" -gt 0 ]; then
    PROCESS_FRACTION=$(awk "BEGIN {printf \"%.6f\", $LAST_TREE_CPU / $CPU_TOTAL_DELTA}")
    ATTRIBUTED_J=$(awk "BEGIN {printf \"%.4f\", ($RAPL_DELTA_UJ / 1000000.0) * ($LAST_TREE_CPU / $CPU_TOTAL_DELTA)}")
else
    PROCESS_FRACTION="0.000000"
    ATTRIBUTED_J="0.0000"
fi

CPU_COUNT=$(nproc)

# ── output ───────────────────────────────────────────────────────────────────

cat <<EOF
{
  "method": "raw_rapl_baseline",
  "workload_pid": $WL_PID,
  "duration_seconds": $DURATION,
  "cpu_count": $CPU_COUNT,
  "rapl_total_energy_j": $RAPL_TOTAL_J,
  "process_cpu_jiffies": $LAST_TREE_CPU,
  "system_cpu_jiffies_delta": $CPU_TOTAL_DELTA,
  "process_fraction": $PROCESS_FRACTION,
  "attributed_energy_j": $ATTRIBUTED_J,
  "poll_count": $POLL_COUNT
}
EOF
