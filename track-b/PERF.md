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
| Phase 1 | Memory | TBD | TBD | TBD | Remove global `pairs` Vec, thread-local direct bitmaps/sparse arenas |
| Phase 2 | Tokenizer | TBD | TBD | TBD | AVX2 SIMD scanning & delimiter detection |
| Phase 3 | Prefix | TBD | TBD | TBD | 64K query-prefix lookup / fast rejection |
| Phase 4 | AC | TBD | TBD | TBD | Direct array root-state transition in Aho-Corasick |
| Phase 5 | Boolean | TBD | TBD | TBD | SIMD bitwise ops and hardware popcount |
| Phase 6 | Output | TBD | TBD | TBD | Preallocated buffer and branchless itoa |

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

## Notes

- Before step 1 was pushed, the BOOL answers took 33 s on a synthetic full-size corpus: intersecting two 250,000-document lists by binary search costs 20-30 ms, and chains like `the AND in AND of` repeat it. Bitmaps for every term in more than N/32 documents brought the answer phase to 0.35 s.
- Step 1 laptop profile (8 threads, synthetic 250,000 documents, 50,000 queries): read 150 ms (Windows), scan 600 ms, postings 60 ms, phrase hits 100 ms, answers 350 ms.
- Checked against a reimplementation of the reference on the spec's worked example and 240 random corpora (upper case, non-ASCII separators, punctuation joins, repeated runs, over-long tokens, empty documents, left-to-right operator chains, scrambled and repeated phrases, absent terms).
- Step 1 was re-run as a side effect of a later push (run-0ac3ba26b183): 0.590 s, 310.7 MB, 51.8 points, same code; the board's spread between runs of identical code is visible here. Best measured board run is `run-1e45bc9e0c74` (0.5245 s, 311.1 MB, K=11.5155, 53.06 points).
