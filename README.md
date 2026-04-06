# wc-rs

A fast `wc` (word count) replacement written in Rust, with AVX2 SIMD acceleration and multi-file parallelism.

## Performance

Benchmarked on a 100MB text corpus (12-core machine):

| Mode | GNU wc | cw (Rust) | wc-rs | vs GNU wc |
|------|--------|-----------|-------|-----------|
| Default (-lwc) | 278 MB/s | 555 MB/s | **7,490 MB/s** | **26.9x** |
| Words (-w) | 280 MB/s | 561 MB/s | **6,991 MB/s** | **24.9x** |
| Lines (-l) | 8,738 MB/s | 6,991 MB/s | **9,533 MB/s** | **1.1x** |
| Chars (-m) | 294 MB/s | N/A | **6,168 MB/s** | **21x** |
| Bytes (-c) | fstat | fstat | **fstat** | tied |

Multi-file (1000 files, 53MB total):

| GNU wc | cw | wc-rs |
|--------|-----|-------|
| 191ms | 102ms | **11ms** |

## How it works

- **AVX2 SIMD word counting**: Classifies 32 bytes at a time using `cmpgt` + `movemask`, detects word boundaries as not-word-to-word transitions in the bitmask, counts with `popcount`. Falls back to SSE2 on older CPUs.
- **Fused line+word counting**: Lines and words counted in a single AVX2 pass over the data, avoiding redundant memory reads.
- **AVX2 char counting**: Counts non-continuation bytes (`b > 0xBF` signed) with a single comparison per 32 bytes.
- **mmap**: Regular files are memory-mapped to avoid kernel-to-userspace copies.
- **rayon parallelism**: Multiple files are counted in parallel across all CPU cores.

## Install

```
cargo install --path .
```

Or build from source:

```
cargo build --release
./target/release/wc-rs --help
```

## Usage

```
wc-rs [OPTIONS] [FILES]...
```

Drop-in replacement for GNU `wc`. Supports the same flags:

| Flag | Description |
|------|-------------|
| `-l` | Print line count |
| `-w` | Print word count |
| `-c` | Print byte count |
| `-m` | Print character count |
| `-L` | Print maximum line length |

With no flags, prints lines, words, and bytes (same as `-lwc`).

```sh
# Count lines, words, bytes
wc-rs file.txt

# Count only lines
wc-rs -l *.log

# Read from stdin
cat file.txt | wc-rs

# Multiple files (parallel)
wc-rs -lw *.txt
```

## Correctness

Output matches GNU wc on well-formed text inputs. Verified with 13 parity tests covering:
- Empty files, single/multi-line, no trailing newline
- UTF-8 text (including combining marks and wide characters for `-L`)
- Binary data, control characters
- Tab handling for max-line-length
- Multi-file totals, stdin input

Known limitation: `-m` counts non-continuation bytes, which differs from GNU wc's `mbrtowc()` on malformed UTF-8 sequences. For valid UTF-8 the results are identical.

## How the optimizations were discovered

This project was developed using an autoresearch loop -- an autonomous experiment process that systematically tries optimizations, measures them, and keeps only what improves throughput.

Key findings from 12 experiments:
- **Branchless word counting is slower** (-28%). The CPU branch predictor is highly effective on natural text. Double table lookups create data dependency chains that prevent speculation.
- **SIMD is the only path to 10x+**. Processing 32 bytes at a time with AVX2 bypasses the per-byte bottleneck entirely.
- **Fusing counts matters**. Counting lines and words in the same SIMD pass is 36% faster than two separate passes, because it halves the memory bandwidth requirement.

## License

MIT
