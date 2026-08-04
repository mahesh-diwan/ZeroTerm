#!/usr/bin/env bash
# ZeroTerm v1.0 release-gate validation: startup time, idle memory, frame-rate
# readiness. Companion to scripts/bench.sh (parser throughput); this one
# measures the shipped GUI binary. Gate rationale: docs/perf.md.
set -euo pipefail

cd "$(dirname "$0")/.."

CARGO="${CARGO:-cargo}"
BIN="target/release/zeroterm"

REBUILD=0
[ "${1:-}" = "--rebuild" ] && REBUILD=1

# v1.0 release gates
START_COLD_MS=200  # cold start < 200ms
START_WARM_MS=50   # warm start  < 50ms
RSS_KB_LIMIT=50000 # idle RSS   < 50MB

LOG_FILE=""
cleanup() {
	[ -n "$LOG_FILE" ] && rm -f "$LOG_FILE"
}
trap cleanup EXIT

if [ ! -x "$BIN" ] || [ "$REBUILD" -eq 1 ]; then
	echo "== Building release binary =="
	"$CARGO" build --release -p zeroterm
fi

# Launch the app, wait for the renderer's "Renderer initialized" readiness log,
# then stop it. Success = found; failure = ERR (readiness never appeared).
# Record the bash-launched pid so we can kill only our own instance.
measure_startup_ms() {
	local log pid start_ms end_ms found
	log=$(mktemp)
	LOG_FILE="$log"
	start_ms=$(date +%s%3N)
	RUST_LOG=info "$BIN" >"$log" 2>&1 &
	pid=$!
	found=0
	for _ in $(seq 1 600); do
		if ! kill -0 "$pid" 2>/dev/null; then break; fi
		if grep -q "Renderer initialized" "$log" 2>/dev/null; then
			found=1
			break
		fi
		sleep 0.05
	done
	end_ms=$(date +%s%3N)
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	rm -f "$log"
	LOG_FILE=""
	if [ "$found" -eq 1 ]; then
		echo "$((end_ms - start_ms))"
	else
		echo "ERR"
	fi
}

# Launch, wait for readiness, let renderer + shell settle, then sample peak idle
# RSS (ps -o rss= reports VmRSS in kB).
measure_rss_kb() {
	local log pid found rss sample
	log=$(mktemp)
	LOG_FILE="$log"
	RUST_LOG=info "$BIN" >"$log" 2>&1 &
	pid=$!
	found=0
	for _ in $(seq 1 600); do
		if ! kill -0 "$pid" 2>/dev/null; then break; fi
		if grep -q "Renderer initialized" "$log" 2>/dev/null; then
			found=1
			break
		fi
		sleep 0.05
	done
	rss=0
	if [ "$found" -eq 1 ]; then
		sleep 1
		for _ in 1 2 3; do
			sample=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || true)
			if [ -n "$sample" ] && [ "$sample" -gt "$rss" ]; then
				rss=$sample
			fi
			sleep 0.5
		done
	fi
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	rm -f "$log"
	LOG_FILE=""
	echo "$rss"
}

size=$(du -h "$BIN" 2>/dev/null | cut -f1 || echo "?")
echo "== ZeroTerm v1.0 release gates =="
echo "binary : $BIN ($size)"
echo "method : wall time to \"Renderer initialized\" log; RSS via ps -o rss= (VmRSS, kB)"

echo
echo "== Startup (wall time to renderer ready) =="
cold=$(measure_startup_ms)
sleep 0.5
warm=$(measure_startup_ms)
printf "cold : %s ms (limit %s ms)\n" "$cold" "$START_COLD_MS"
printf "warm : %s ms (limit %s ms)\n" "$warm" "$START_WARM_MS"

echo
echo "== Idle memory =="
rss=$(measure_rss_kb)
rss_mb=$(awk -v r="$rss" 'BEGIN { printf "%.1f", r / 1024 }' 2>/dev/null || echo "?")
printf "RSS  : %s kB (%s MB) (limit %s kB)\n" "$rss" "$rss_mb" "$RSS_KB_LIMIT"

echo
echo "== Results =="
fail=0
pass() { printf "  %-6s PASS\n" "$1"; }
fail_line() {
	printf "  %-6s FAIL (measured %s %s, limit %s)\n" "$1" "$2" "$3" "$4"
	fail=1
}

if [ "$cold" = "ERR" ] || [ -z "$cold" ]; then
	echo "  START  FAIL (cold: app never reached readiness)"
	fail=1
elif [ "$cold" -le "$START_COLD_MS" ]; then
	pass "START"
else
	fail_line "START" "$cold" "ms" "$START_COLD_MS ms"
fi

if [ "$warm" = "ERR" ] || [ -z "$warm" ]; then
	echo "  WARM   FAIL (warm: app never reached readiness)"
	fail=1
elif [ "$warm" -le "$START_WARM_MS" ]; then
	pass "WARM"
else
	fail_line "WARM" "$warm" "ms" "$START_WARM_MS ms"
fi

if [ -z "$rss" ] || [ "$rss" -eq 0 ]; then
	echo "  MEM    FAIL (RSS sample empty)"
	fail=1
elif [ "$rss" -le "$RSS_KB_LIMIT" ]; then
	pass "MEM"
else
	fail_line "MEM" "$rss" "kB" "$RSS_KB_LIMIT kB"
fi

echo "  FPS    NOTE  Frame gate (120 FPS @ 4K) is manual: GPU/vblank-bound."
printf "                  Run zeroterm on a 120Hz 4K display; profile wgpu frame\n"
printf "                  count or hyperfine --fps. Renderer cost model in docs/perf.md.\n"

echo
if [ "$fail" -eq 1 ]; then echo "Gates: FAIL($fail)"; else echo "Gates: PASS (frame gate is manual)"; fi
