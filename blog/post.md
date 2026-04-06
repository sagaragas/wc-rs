# Rewriting `wc` in Rust: From 280 MB/s to 7.5 GB/s

**TL;DR**: I rewrote `wc` in Rust and used an autonomous experiment loop to discover optimizations. The result is 27x faster than GNU coreutils `wc` on default mode and 13x faster than `cw`, the existing Rust alternative. The key insight: branchless scalar is a dead end, but AVX2 SIMD with fused multi-metric counting gets you to memory bandwidth.

## Why wc?

`wc` is deceptively simple. Count lines, words, and bytes. But it's also one of those Unix tools that gets used on multi-gigabyte log files, and the difference between 300 MB/s and 7 GB/s is the difference between waiting and not waiting.

GNU coreutils `wc` is already optimized. It has separate code paths for `-l` (line-only, uses a fast loop), `-c` (byte-only, just calls `fstat`), and the full `-lwc` mode. There was even a proposed AVX2 patch for line counting that was debated on the mailing list. And `cw` by Freaky is a well-known Rust alternative that uses threading.

The optimization landscape is well-understood. That made it a perfect target for systematic exploration.

## The approach: autoresearch

Instead of guessing at optimizations, I used an autoresearch loop: make a change, measure throughput on a 100MB corpus, keep improvements, discard regressions. Every experiment is logged with its hypothesis, result, and confidence score. Dead ends get recorded so they're never revisited.

I ran 12 experiments over one session. Here's the full trajectory.

## The baseline: 517 MB/s

The naive Rust implementation is straightforward:

```rust
for &b in data {
    if b == b'\n' { lines += 1; }
    if is_whitespace(b) {
        in_word = false;
    } else if is_word_byte(b) && !in_word {
        in_word = true;
        words += 1;
    }
}
```

With a 64K buffered reader, this clocks in at 517 MB/s -- already 1.6x faster than GNU wc (319 MB/s). Rust's iterator optimizations and LLVM's codegen are doing some work here, but the algorithm is identical.

The parity surface is critical: output must match GNU wc exactly. I wrote 13 parity tests that compare output on empty files, UTF-8 text, binary data, control characters, tabs, and multi-file totals. Every experiment runs these tests before benchmarking.

## Incremental scalar gains: 517 -> 628 MB/s

Three small wins stacked:

1. **Lookup table (+7.4%)**: Replace the branch-heavy `is_whitespace()` and `is_word_byte()` checks with a single 256-byte lookup table. One memory access instead of 6 comparisons.

2. **Larger buffers (+5.6%)**: 64K to 256K. Fewer syscalls, better cache utilization.

3. **memchr for newlines (+5.9%)**: The `memchr` crate uses SIMD internally. Splitting the loop into a SIMD newline pass and a scalar word pass is faster than checking newlines in the scalar loop.

4. **mmap (+1.2%)**: Memory-mapping files avoids the kernel-to-userspace copy. Marginal for the word-counting bottleneck but noticeable on the line-counting path.

Total: 517 -> 628 MB/s. Decent, but still in the same ballpark as GNU wc.

## The dead end: branchless word counting

Here's where it gets interesting. The word state machine has branches:

```
if whitespace -> leave word
else if word_byte && !in_word -> enter word, count++
else -> no change
```

The textbook optimization is to make this branchless. I tried three variants:

- **Two-table lookup** (BYTE_CLASS + TRANSITION): -28%
- **Fused 512-byte single-lookup table**: -30%
- **Scalar bitmask building**: -25%

All slower. Significantly slower.

The reason: the CPU's branch predictor is extremely effective on natural text. Words average ~5 characters, spaces average ~1. The predictor learns "stay in current state" and gets it right ~84% of the time. Branchless approaches replace well-predicted branches with guaranteed data dependency chains -- every byte must wait for the previous byte's result before it can proceed.

This is a fundamental insight that the autoresearch loop surfaced cleanly. Three experiments, three rejections, with clear rollback reasons logged. A human optimizer might have spent days pursuing branchless approaches. The data said no in 10 minutes.

## The breakthrough: SIMD (628 -> 7,490 MB/s)

If you can't make each byte faster, process more bytes at once.

**SSE2 (16 bytes at a time): 628 -> 2,231 MB/s**

The approach:
1. Load 16 bytes into an SSE2 register
2. Classify each byte as "word byte" using `cmpgt(v, 0x20)` (catches printable ASCII) OR'd with a UTF-8 lead byte range check
3. Extract to a 16-bit integer with `movemask`
4. Detect word starts: `word_starts = (~word_bits << 1 | carry) & word_bits` -- a word starts where the current byte is a word byte and the previous wasn't
5. Count with `popcount`

This is 3.5x faster than scalar because it replaces per-byte branches with bulk SIMD classification.

**AVX2 (32 bytes at a time): 2,231 -> 5,519 MB/s**

Same algorithm, double the width. Since this machine has AVX2, the upgrade is nearly linear.

**Fused line+word counting: 5,519 -> 7,490 MB/s**

The last 36% came from fusing line counting into the AVX2 loop. Previously, lines were counted with a separate `memchr` pass and words with the SIMD pass -- two reads of the same data. Fusing them into one loop (add a `cmpeq` for newlines alongside the word detection) cuts the memory bandwidth requirement in half.

At 7,490 MB/s, we're approaching memory bandwidth on this machine. There's not much headroom left for single-file throughput.

## Character counting: the free SIMD win

GNU wc's `-m` (character counting) runs at 294 MB/s because it processes bytes through `mbrtowc()`. But UTF-8 character counting is trivially SIMD-able: a character is any byte that's NOT a continuation byte (0x80-0xBF). In AVX2:

```rust
let threshold = _mm256_set1_epi8(-65i8); // 0xBF as signed
let non_continuation = _mm256_cmpgt_epi8(v, threshold);
chars += movemask(non_continuation).count_ones();
```

One comparison, one movemask, one popcount per 32 bytes. Result: 5,243 MB/s, or **17.8x GNU wc**.

## Multi-file parallelism

For `wc *.log` on 1000 files, rayon parallelism across 12 cores gives:

| Tool | Time |
|------|------|
| GNU wc | 191ms |
| cw | 102ms |
| wc-rs | **11ms** |

Each file gets its own `mmap` + AVX2 counting, distributed across the thread pool.

## Final scoreboard

100MB text corpus, single file:

| Mode | GNU wc | cw (Rust) | wc-rs | vs GNU |
|------|--------|-----------|-------|--------|
| Default | 278 MB/s | 555 MB/s | **7,490 MB/s** | **27x** |
| Words | 280 MB/s | 561 MB/s | **6,991 MB/s** | **25x** |
| Lines | 8,738 MB/s | 6,991 MB/s | **9,533 MB/s** | 1.1x |
| Chars | 294 MB/s | -- | **5,243 MB/s** | **18x** |

## What I learned

1. **Measure before optimizing.** The naive Rust implementation was already 1.6x faster than GNU wc. Not every C program is faster than its Rust equivalent.

2. **Branch prediction defeats branchless on predictable data.** Three separate branchless experiments all regressed. The data dependency chain matters more than the branch misprediction rate.

3. **SIMD is a different game.** 32 bytes per cycle isn't an incremental improvement over 1 byte per cycle. It's a category change. The algorithmic approach is completely different -- bitmasks and popcount instead of state machines.

4. **Fuse your passes.** Two SIMD passes over the same data at memory bandwidth is no faster than one. The bottleneck shifted from computation to memory access.

5. **Autonomous experimentation works.** 12 experiments in one session, with clear keep/discard decisions, complete rollback on failure, and a structured log of what was tried. No wasted time revisiting dead ends.

The code is at [github.com/user/wc-rs](https://github.com/user/wc-rs). 680 lines of Rust, 13 parity tests, and a benchmark harness you can reproduce.
