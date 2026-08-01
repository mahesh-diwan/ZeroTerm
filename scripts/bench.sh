#!/usr/bin/env bash
# Run ZeroTerm parser benchmarks and summarize 4K@120fps headroom.
set -euo pipefail

cd "$(dirname "$0")/.."

echo "== Running parser benchmarks (--quick) =="
out="$(cargo bench -p zeroterm-core --bench parser_bench -- --quick 2>&1)"

# The render need: 480x135 cells @ 120fps = 7,776,000 cells/s.
required=$((480 * 135 * 120))

echo
echo "== Headroom vs 4K@120fps (need ${required} cells/s) =="
printf "%-38s %8s %12s %8s\n" "bench" "MiB/s" "cells/s" "headroom"
echo "----------------------------------------------------------------------"
group=""
while IFS= read -r line; do
	if g=$(grep -oE '^[A-Za-z0-9_]+/[A-Za-z0-9_]+' <<<"$line" | head -1) && [ -n "$g" ]; then
		group="${g%/*}"
		group="${group#parse_}"
		group="${group#screen_}"
	fi
	mib=$(grep -oE '[0-9.]+ MiB/s' <<<"$line" | head -1 | cut -d' ' -f1 || true)
	if [ -n "$mib" ] && [ -n "$group" ]; then
		bytes_per_sec=$(awk -v m="$mib" 'BEGIN { printf "%.0f", m * 1048576 }')
		cells=$(awk -v b="$bytes_per_sec" 'BEGIN { printf "%.0f", b / 2 }')
		hr=$(awk -v c="$cells" -v r="$required" 'BEGIN { printf "%.1fx", c / r }')
		printf "%-38s %8s %12s %8s\n" "$group" "$mib" "$cells" "$hr"
		group=""
	fi
done <<<"$out"
echo
echo "All realistic workloads must clear ${required} cells/s. See docs/perf.md."
