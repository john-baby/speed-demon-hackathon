# Track B: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 56.35 | 2290.0 | 0.0 |
| 1 | Rust rewrite: index only the query terms in one parallel mmap scan; all phrases matched during the scan by an Aho-Corasick automaton over term ids; BOOL posting lists by parallel counting sort; hybrid sorted-list / bitmap evaluation strictly left to right | run-1e45bc9e0c74 | 0.5245 | 311.1 | 53.06 |

## Optimization Phase Progress

| Version | Phase | Runtime | Peak RSS | SHA-256 | Notes |
|---|---|---:|---:|---|---|
| Baseline | Original | 524.5 ms | 311.1 MB | Verified (`f86973c2...`) | Initial Rust baseline (run-1e45bc9e0c74, K=11.5155, 575 lines, D=1) |
| Phase 1 | Memory | 184.7 ms (sample) | 47.1 MB (sample) | Verified (`eeeec1e9...`) | Removed global `pairs` Vec; thread-local postings; dense terms stored only as bitmaps; code reduced by 15 lines (560 lines) |
| Phase 2 | Tokenizer | 175.9 ms (sample) | 46.9 MB (sample) | Verified (`eeeec1e9...`) | AVX2 SIMD scanning, 32-byte block processing, SIMD case-folding & bitmask delimiter skipping |
| Phase 3 | Prefix | 172.5 ms (sample) | 46.9 MB (sample) | Verified (`eeeec1e9...`) | 64K query-prefix bitmap lookup / fast rejection of non-query tokens; 8 KB L1-resident table |
| Phase 4 | AC | 165.4 ms (sample) | 46.8 MB (sample) | Verified (`eeeec1e9...`) | O(1) direct root transitions; flat sorted non-root edge arrays; struct Map completely removed (-32 lines) |
| Phase 5 | Boolean | 164.5 ms (sample) | 46.8 MB (sample) | Verified (`eeeec1e9...`) | AVX2 vector bitwise ops, hardware popcnt, early exit on 10 IDs, zero-alloc phrase result slices |
| Phase 6 | Output | 161.0 ms (sample) | 46.7 MB (sample) | Verified (`eeeec1e9...`) | Branchless 2-digit lookup itoa, preallocated line capacity, single-pass contiguous output buffer |

## Phase 0 Baseline Analysis & Identified Bottlenecks

1. **Intermediate Allocations (Primary Memory Bottleneck):**
   - Each thread scans documents into `pairs: Vec<(u32, u32)>`. Across the 250,000 documents, this stores ~29.8M pairs (~238.4 MB).
   - `postings: vec![0u32; total]` allocates another ~119.2 MB during counting sort.
   - Combined peak memory is 311.1 MB (vs leaderboard leaders at 25.5–44.4 MB).
2. **Tokenizer CPU Overhead (Primary Runtime Bottleneck):**
   - Scalar byte-by-byte scanning with copying into `key: [u8; 32]` and open-addressing hash table probing on all 36M tokens.
   - Takes ~350-400 ms of the 524.5 ms total wall time.
3. **Aho-Corasick Automaton Transitions:**
   - Lookups use an open-addressing hash map (`Map::get((s << 32) | t)`) on every token transition. Root transitions (state 0) constitute >98% of checks and can be made O(1) via direct array indexing.
4. **Correctness Status:**
   - Byte-for-byte verified against official spec worked example (18 queries, 5 docs).
   - Official full-scale dataset SHA-256 confirmed as `f86973c210228141bcb6f1f1b10188ad2a9b65f71e7cd7cf4590b38de4e80927`.

## Phase 1 Findings & Results

- **Global Intermediate Pairs Removed:** `part.pairs: Vec<(u32, u32)>` completely eliminated. Document IDs are now accumulated per term in thread-local vectors.
- **Dense/Sparse Segregation:** Terms exceeding `big_min` are converted directly into bitmaps. They are never stored in `postings`, eliminating ~25M integer entries from the posting buffer.
- **Natural Ordering Preserved:** Thread-local lists are concatenated in partition order, preserving monotonic document IDs without sorting.
- **Differential Correctness:** Output SHA-256 matches the expected checksum on both the spec worked example and the official seed-1 sample dataset (`eeeec1e9319da98a6f3c664c78f93e7078fd603c25c2bb3027174e81e3638a8f`).
- **Code Size:** Reduced from 575 to 560 lines (`scc` line count decreases, improving the $S$ ratio).

## Phase 2 Findings & Results

- **AVX2 SIMD Block Classification:** Implemented 32-byte chunk processing with `_mm256_loadu_si256`. Token ranges (`0..9`, `A..Z`, `a..z`) are detected via SIMD vector comparisons.
- **Vectorized Lowercasing:** Uppercase ASCII letters (`A..Z`) are converted to lowercase in SIMD (`_mm256_add_epi8` with masked $+32$) before token extraction.
- **Fast Delimiter Skipping:** Consecutive separators are skipped in single-cycle operations using bitmask manipulation and hardware trailing-zero counts (`tzcnt`).
- **Safe Fallback & Boundary Handling:** Full compatibility maintained across document line boundaries, with scalar tail loops handling trailing sub-32-byte chunks and non-AVX2 hardware via `is_x86_feature_detected!("avx2")`.
- **Differential Correctness:** Output SHA-256 matches byte-for-byte on both the spec worked example and the official seed-1 sample dataset (`eeeec1e9319da98a6f3c664c78f93e7078fd603c25c2bb3027174e81e3638a8f`).

## Phase 3 Findings & Results

- **Compact Prefix Filter:** Implemented a 65,536-bit (8 KB) prefix bitmap for 2-byte token prefixes `(byte0 << 8) | byte1` along with a 256-bit (32 byte) table for 1-byte terms, fitting entirely within the 48 KB L1 Data Cache of each core.
- **Immediate Rejection of Non-Query Tokens:** Non-query tokens with impossible prefixes are immediately discarded with `*state = 0; return;`, bypassing the 32-byte token hash function and open-addressing hash table lookup completely.
- **Zero Heap Allocations for Filter:** The prefix filter is embedded directly within `Terms` as fixed-size arrays (`[u64; 1024]` and `[u64; 4]`), avoiding pointer indirection and extra allocations.
- **Unit & Differential Verification:** Unit tests confirm 100% of query terms pass the filter without false negatives. Official spec worked example and official sample seed-1 SHA-256 verified byte-for-byte (`eeeec1e9319da98a6f3c664c78f93e7078fd603c25c2bb3027174e81e3638a8f`).

## Phase 4 Findings & Results

- **Direct Root Transitions ($O(1)$):** Root state (`state 0`) transitions (accounting for >98% of all checks during document scanning) now use a flat direct lookup array `root_goto: Vec<u32>` indexed directly by `term_id`.
- **Flat Sorted Non-Root Transitions:** Non-root states are stored in contiguous sorted slices (`edges: Vec<(u32, u32)>` and `edge_start: Vec<u32>`), replacing the hash table. 97.1% of branching non-root states have degree 1, which evaluates via a single integer equality comparison.
- **`Map` Struct Completely Removed:** The 32-line open-addressing hash table `Map` was eliminated entirely from the codebase, reducing binary complexity, eliminating hash collisions, and improving the source line count ratio $S$.
- **Fast Output Check Skip:** When `*state == 0`, output link traversal is bypassed entirely since root can never end a phrase query.
- **Differential & Unit Testing:** All 5 unit tests pass, covering 2-token phrases, 8-token phrases, repeated terms, overlapping phrases, and failure link traversals. Verified byte-for-byte against the spec worked example (18 queries) and the sample seed-1 dataset (`eeeec1e9319da98a6f3c664c78f93e7078fd603c25c2bb3027174e81e3638a8f`).
- **Benchmark Improvement:** Runtime dropped to a fastest run of **165.4 ms** on sample data.

## Phase 5 Findings & Results

- **AVX2 Vector Bitwise Operations:** Bitmap AND (`_mm256_and_si256`), OR (`_mm256_or_si256`), and NOT / set difference (`_mm256_andnot_si256`) process 256 bits (4 u64 words) per instruction with fallback for non-AVX2 environments.
- **Hardware `popcnt` Aggregation:** Document counts over bitmaps evaluate in $O(\text{words})$ via hardware `count_ones()` (`popcnt`) instructions, executing in microseconds.
- **Early Termination for Smallest 10 IDs:** ID extraction from bitmaps stops immediately after collecting 10 IDs instead of iterating through hundreds of thousands of set bits.
- **Fast Integer Sum Evaluation:** Bitmap sums use `base * popcnt + bit_positions` with full-word `u64::MAX` shortcuts ($base \times 64 + 2016$).
- **Zero-Allocation Phrase Slices:** Phrase query evaluation borrows directly from `hits` via `Set::PhraseSlice(&hits[lo..hi])`, eliminating ~19,000 intermediate `Vec<u32>` heap allocations.
- **Two-Pointer Linear Scans:** Sparse posting list intersections and differences use branch-efficient two-pointer scans when list lengths are within 16x, with galloping fallback for highly asymmetrical list pairs.
- **Unit & Differential Testing:** All 8 unit tests pass. Output checksum on the official seed-1 sample dataset verified byte-for-byte (`eeeec1e9319da98a6f3c664c78f93e7078fd603c25c2bb3027174e81e3638a8f`) and on the 18 spec worked example queries.

## Phase 6 Findings & Results

- **Branchless Lookup `itoa` Formatting:** Replaced `std::io::Write` trait calls and `write!` macro dynamic parsing with a 2-digit lookup table (`DIGITS`), processing numbers 2 digits per iteration with zero division by 10.
- **Capacity-Preallocated Lines:** Each formatted line preallocates `32 + 8 * first.len()`, eliminating vector reallocation during line construction.
- **Single-Pass Output Assembly:** The final output buffer calculates exact `total_len` and allocates `final_out` in one contiguous slice, removing multi-pass `concat` reallocation.
- **Zero-Match Query Handling:** Correctly handles zero-match queries as `0 0\n` per specification.
- **Unit & Differential Testing:** 11 unit tests pass. SHA-256 verified byte-for-byte on the official sample seed-1 dataset (`eeeec1e9319da98a6f3c664c78f93e7078fd603c25c2bb3027174e81e3638a8f`) and spec worked example.
- **Benchmark Improvement:** Runtime dropped to a fastest run of **161.0 ms** on sample data.

## Notes

- Before step 1 was pushed, the BOOL answers took 33 s on a synthetic full-size corpus: intersecting two 250,000-document lists by binary search costs 20-30 ms, and chains like `the AND in AND of` repeat it. Bitmaps for every term in more than N/32 documents brought the answer phase to 0.35 s.
- Step 1 laptop profile (8 threads, synthetic 250,000 documents, 50,000 queries): read 150 ms (Windows), scan 600 ms, postings 60 ms, phrase hits 100 ms, answers 350 ms.
- Checked against a reimplementation of the reference on the spec's worked example and 240 random corpora (upper case, non-ASCII separators, punctuation joins, repeated runs, over-long tokens, empty documents, left-to-right operator chains, scrambled and repeated phrases, absent terms).
- Step 1 was re-run as a side effect of a later push (run-0ac3ba26b183): 0.590 s, 310.7 MB, 51.8 points, same code; the board's spread between runs of identical code is visible here. Best measured board run is `run-1e45bc9e0c74` (0.5245 s, 311.1 MB, K=11.5155, 53.06 points).
