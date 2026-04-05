#!/bin/bash
# Benchmark wc-rs vs GNU wc
# Usage: ./bench/benchmark.sh [corpus_file]
set -e

WC_RS="$(dirname "$0")/../target/release/wc-rs"
CORPUS="${1:-$(dirname "$0")/data/corpus_100m.txt}"
ITERATIONS="${2:-5}"

if [ ! -f "$WC_RS" ]; then
    echo "Building wc-rs in release mode..."
    (cd "$(dirname "$0")/.." && . "$HOME/.cargo/env" && cargo build --release 2>&1)
fi

if [ ! -f "$CORPUS" ]; then
    echo "Corpus not found: $CORPUS"
    echo "Run: bash bench/generate_corpus.sh"
    exit 1
fi

FILE_SIZE=$(stat -c%s "$CORPUS" 2>/dev/null || stat -f%z "$CORPUS" 2>/dev/null)
FILE_MB=$(echo "scale=1; $FILE_SIZE / 1048576" | bc)

echo "=== wc benchmark ==="
echo "Corpus: $CORPUS (${FILE_MB}MB)"
echo "Iterations: $ITERATIONS"
echo ""

# Warm filesystem cache
cat "$CORPUS" > /dev/null

run_bench() {
    local label="$1"
    shift
    local cmd=("$@")
    local total_ms=0
    local min_ms=999999999

    for i in $(seq 1 "$ITERATIONS"); do
        # Drop caches if possible (ignore errors)
        sync 2>/dev/null
        
        local start_ns=$(date +%s%N)
        "${cmd[@]}" "$CORPUS" > /dev/null
        local end_ns=$(date +%s%N)
        local elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
        total_ms=$((total_ms + elapsed_ms))
        if [ "$elapsed_ms" -lt "$min_ms" ]; then
            min_ms=$elapsed_ms
        fi
    done

    local avg_ms=$((total_ms / ITERATIONS))
    local throughput_mbs=$(echo "scale=1; $FILE_SIZE / ($min_ms * 1000)" | bc 2>/dev/null || echo "N/A")
    printf "%-20s avg: %6dms  min: %6dms  throughput: %s MB/s\n" "$label" "$avg_ms" "$min_ms" "$throughput_mbs"
}

echo "--- Default mode (lines, words, bytes) ---"
run_bench "GNU wc" wc
run_bench "wc-rs" "$WC_RS"

echo ""
echo "--- Line counting only (-l) ---"
run_bench "GNU wc -l" wc -l
run_bench "wc-rs -l" "$WC_RS" -l

echo ""
echo "--- Word counting only (-w) ---"
run_bench "GNU wc -w" wc -w
run_bench "wc-rs -w" "$WC_RS" -w

echo ""
echo "--- Byte counting only (-c) ---"
run_bench "GNU wc -c" wc -c
run_bench "wc-rs -c" "$WC_RS" -c

echo ""
echo "--- Char counting (-m) ---"
run_bench "GNU wc -m" wc -m
run_bench "wc-rs -m" "$WC_RS" -m

echo ""
echo "--- Max line length (-L) ---"
run_bench "GNU wc -L" wc -L
run_bench "wc-rs -L" "$WC_RS" -L

echo ""
echo "--- Parity check ---"
GNU_OUT=$(wc "$CORPUS")
OUR_OUT=$("$WC_RS" "$CORPUS")
if [ "$GNU_OUT" = "$OUR_OUT" ]; then
    echo "PASS: Output matches GNU wc"
else
    echo "FAIL: Output mismatch"
    echo "  GNU: $GNU_OUT"
    echo "  Ours: $OUR_OUT"
fi
