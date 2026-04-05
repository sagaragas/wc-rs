#!/bin/bash
# Generate benchmark corpus files
set -e

BENCH_DIR="$(dirname "$0")/data"
mkdir -p "$BENCH_DIR"

# ~100MB text file from /dev/urandom mapped to printable ASCII with newlines
echo "Generating 100MB corpus..."
python3 -c "
import random
import string
chars = string.ascii_letters + string.digits + ' ' * 6
with open('$BENCH_DIR/corpus_100m.txt', 'w') as f:
    line_len = 0
    for _ in range(100 * 1024 * 1024):
        if line_len > 60 + random.randint(0, 40):
            f.write('\n')
            line_len = 0
        else:
            f.write(random.choice(chars))
            line_len += 1
    f.write('\n')
"

echo "Generating 10MB corpus..."
head -c $((10 * 1024 * 1024)) "$BENCH_DIR/corpus_100m.txt" > "$BENCH_DIR/corpus_10m.txt"

echo "Generating 1MB corpus..."
head -c $((1 * 1024 * 1024)) "$BENCH_DIR/corpus_100m.txt" > "$BENCH_DIR/corpus_1m.txt"

echo "Done. Files:"
ls -lh "$BENCH_DIR/"
