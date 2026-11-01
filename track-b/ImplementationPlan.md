# SPEED DEMONS — TRACK B OPTIMIZATION IMPLEMENTATION PLAN

**Project:** Speed Demons  
**Track:** B — Index  
**Team:** Team7  
**Language:** Rust  
**AI Coding Agent:** Antigravity — Gemini Flash Turbo  
**Development Environment:** Windows + WSL2 Ubuntu + Docker  
**Primary Objective:** Maximize the Track B efficiency coefficient (K).

---

# 1. MANDATORY INSTRUCTIONS

This document is the source of truth for the entire Track B optimization task.

The AI coding agent MUST read this entire document before modifying any source code.

Follow every implementation phase sequentially.

## Non-negotiable rules

1. Do not skip implementation phases.
2. Do not implement all phases simultaneously.
3. Do not rewrite the entire application unnecessarily.
4. Preserve the existing working implementation.
5. Run correctness tests after every meaningful change.
6. Benchmark every optimization.
7. Commit every successfully completed phase separately.
8. Never overwrite a verified working implementation without preserving a rollback path.
9. Never modify official expected checksums.
10. Never claim a performance improvement without an actual benchmark.
11. Maintain zero external dependencies.
12. Never use banned full-text indexing or search libraries.
13. Do not push changes to remote repositories without explicit authorization.
14. Do not use destructive Git commands without explicit authorization.
15. Do not sacrifice correctness for benchmark performance.

If a phase fails its acceptance criteria, fix it before proceeding.

---

# 2. CURRENT PROJECT BASELINE

Existing implementation:

`track-b/src/main.rs`

Current architecture:

- Rust single-binary implementation.
- Aho-Corasick phrase matching.
- Query-driven term extraction.
- Parallel document scanning.
- Thread-local `(term_id, doc_id)` pairs.
- Hybrid posting-list and bitmap representation.
- Boolean query evaluation.
- Standard-library-only implementation.

## Current benchmark

| Metric | Baseline |
|---|---:|
| Runtime | 0.5245 seconds |
| Peak memory | 311.1 MB |
| Efficiency coefficient K | 11.5155 |
| Speed score | 53.06 |
| Source lines | 575 |
| External dependencies | 0 |
| Language | Rust |
| Leaderboard rank | 59 |

## Optimization targets

These are targets, not guaranteed results.

| Metric | Baseline | Intermediate target | Stretch target |
|---|---:|---:|---:|
| Runtime | 524.5 ms | 200 ms | 75–100 ms |
| Memory | 311 MB | 100 MB | 25–45 MB |
| K | 11.52 | 20+ | 40–50+ |
| Dependencies | 0 | 0 | 0 |

The ultimate objective is to surpass the current leaderboard leader.

Do not assume that the stretch targets are achievable until measured.

---

# 3. OFFICIAL TRACK B SPECIFICATIONS

## Hardware

Official execution environment:

- Intel Core i5-13500.
- 6 P-cores and 12 hardware threads.
- Turbo disabled.
- AVX2 enabled.
- AVX-512 unavailable.
- 8 GB RAM.
- Zero swap.
- Linux bare-metal execution.
- Offline runtime.

## Input files

`/data/docs.txt`

- 250,000 documents.
- Approximately 253.8 MB.
- Each line contains `doc_id<TAB>text<NEWLINE>`.
- Document IDs are dense and ascending.
- Empty document text is permitted.

`/data/queries.txt`

- 50,000 queries.
- Approximately 1.76 MB.
- 31,000 Boolean queries.
- 19,000 phrase queries.

Queries are available at process start.

Indexing only terms present in the queries is allowed and expected.

## Tokenization

Implement exactly these rules:

1. Convert ASCII uppercase A–Z to lowercase.
2. Token characters are only `a-z` and `0-9`.
3. Every other byte is a separator.
4. Bytes >= 0x80 are separators.
5. UTF-8 sequences are not decoded or normalized.
6. No stemming.
7. No stopword removal.
8. No Unicode normalization.
9. Maximum token length is 32 bytes.

Examples:

- `HELLO` → `hello`
- `Zürich` → `z`, `rich`
- `naïve` → `na`, `ve`
- `273K` → `273`
- `abc123` → `abc123`

## Boolean semantics

Boolean evaluation is strictly left-to-right.

There is no conventional operator precedence.

Example:

`a OR b AND c`

Must evaluate as:

`(a OR b) AND c`

NOT is binary set difference.

Example:

`a NOT b`

Means:

`A \ B`

Lowercase `and`, `or`, and `not` are ordinary search terms when they occur as terms.

## Phrase semantics

A phrase matches when all query terms occur at consecutive token positions.

Separators are ignored between tokens.

Phrase matches may cross punctuation and sentence boundaries.

Phrase matches must never cross document boundaries.

## Output format

Output file:

`/out/results.txt`

Exactly 50,000 lines.

Each line:

`<count> <sum> <id_1> <id_2> ... <id_m>`

Where:

- Count is the total matching document count.
- Sum is the exact 64-bit sum of matching document IDs.
- At most ten smallest matching IDs are included.
- IDs must be ascending.
- Empty results must be `0 0`.

Official SHA-256:

`f86973c210228141bcb6f1f1b10188ad2a9b65f71e7cd7cf4590b38de4e80927`

The final output must match this checksum byte-for-byte.

---

# 4. SCORING FORMULA

The efficiency coefficient is:

K = (T_ref/T)^0.50 × (M_ref/M)^0.20 × (S_ref/S)^0.15 × (D_ref/D)^0.15

Where:

- T = runtime.
- M = peak memory.
- S = source lines.
- D = 1 + external dependencies.

Reference values:

- T_ref approximately 56.35 seconds.
- M_ref approximately 2.29 GB.
- D_ref = 1.

Important:

- Runtime is uncapped.
- Memory contribution is capped at 50x.
- Source line contribution cannot exceed 1.0.
- Zero external dependencies gives D = 1.

Keep the implementation compact without sacrificing performance or correctness.

---

# 5. GIT VERSION CONTROL POLICY

Git commits are mandatory.

Every successfully completed implementation phase must have its own commit.

## Initial setup

Before modifying files:

1. Inspect `git status`.
2. Inspect the current branch.
3. Inspect existing uncommitted changes.
4. Preserve all existing user work.
5. Create a dedicated branch if one does not already exist:

`git switch -c track-b-optimization`

If the branch already exists, switch to it rather than recreating it.

Create a baseline tag:

`git tag track-b-baseline`

Do not overwrite an existing tag.

Do not commit unrelated user changes.

## Mandatory phase commits

Use these commit messages:

| Phase | Commit message |
|---|---|
| Phase 0 | `chore(track-b): establish baseline benchmarks` |
| Phase 1 | `perf(track-b): implement memory-efficient indexing` |
| Phase 2 | `perf(track-b): optimize tokenizer with AVX2` |
| Phase 3 | `perf(track-b): add query prefix filtering` |
| Phase 4 | `perf(track-b): optimize aho-corasick transitions` |
| Phase 5 | `perf(track-b): optimize boolean query evaluation` |
| Phase 6 | `perf(track-b): optimize output formatting` |
| Final | `test(track-b): verify final correctness and performance` |

## Before every commit

1. Compile in release mode.
2. Run relevant unit tests.
3. Run differential correctness tests.
4. Run checksum verification when applicable.
5. Run performance benchmarks.
6. Record runtime and memory.
7. Inspect `git diff`.
8. Ensure temporary files and unrelated files are excluded.

Only commit when the phase passes its acceptance criteria.

## After every commit

Run:

`git status --short`

`git log -1 --oneline`

Record the commit hash.

Do not squash phase commits.

Do not leave successful implementation phases uncommitted.

## Failure handling

If a phase fails:

- Do not create its successful-phase commit.
- Diagnose the failure.
- Fix the implementation.
- Rerun the tests.
- Benchmark again.
- Commit only after acceptance.

Never use:

- `git reset --hard`
- `git clean -fd`
- Force-push

without explicit authorization.

Do not push to a remote repository.

---

# 6. DEVELOPMENT ENVIRONMENT

Development takes place on Windows using WSL2 Ubuntu and Docker.

## WSL preparation

Install required tools:

`sudo apt update`

`sudo apt install -y build-essential clang llvm hyperfine time`

Install Rust stable if necessary.

Verify:

`rustc --version`

`cargo --version`

`lscpu`

`grep -m1 -o avx2 /proc/cpuinfo`

Check Docker:

`docker --version`

`docker info`

## Build command

From `track-b`:

`cargo build --release`

## Testing command

`cargo test --release`

## Benchmarking

Use repeated measurements rather than a single run.

Record:

- Wall-clock time.
- Peak resident memory.
- CPU utilization where available.
- Exit code.
- Output checksum.

Use identical input data for baseline and optimized versions.

## Docker environment

Create a Dockerfile that:

1. Uses a Linux Rust build environment.
2. Compiles the project in release mode.
3. Copies only the compiled executable into the runtime image.
4. Mounts `/data` read-only.
5. Mounts `/out` writable.

Example runtime command:

`docker run --rm --cpus=12 --memory=8g --mount type=bind,src=/path/to/data,dst=/data,readonly --mount type=bind,src=/path/to/output,dst=/out track-b:test`

Replace paths with actual WSL paths.

Important:

WSL and Docker are local development environments. Their timing and memory measurements may differ from the official scoring machine.

Final acceptance requires the official environment whenever available.

---

# 7. IMPLEMENTATION PHASES

# PHASE 0 — BASELINE INSPECTION

Do not modify the implementation initially.

Tasks:

1. Inspect the complete `track-b` directory.
2. Read the existing `main.rs`.
3. Understand query parsing.
4. Understand term extraction.
5. Understand Aho-Corasick construction.
6. Understand document scanning.
7. Understand parallel indexing.
8. Understand posting-list construction.
9. Understand Boolean evaluation.
10. Understand result formatting.

Identify:

- Largest allocations.
- Hot loops.
- Hash-table lookups.
- Memory copies.
- Thread synchronization.
- Unnecessary sorting.
- Query evaluation costs.

Establish baseline correctness.

Verify the official checksum.

Benchmark the baseline.

Document all findings in `PERF.md`.

Commit:

`chore(track-b): establish baseline benchmarks`

Acceptance:

- Existing implementation understood.
- Baseline reproducible.
- Original source preserved.
- Git state verified.

# PHASE 1 — MEMORY-EFFICIENT INDEXING

Highest priority.

Current major problem:

The implementation stores approximately 29.8 million `(term_id, doc_id)` pairs.

This creates substantial intermediate memory usage.

Objective:

Remove the global intermediate pairs architecture.

## Implementation

1. Partition documents into contiguous ranges.
2. Assign ranges to worker threads.
3. Build thread-local indexing structures.
4. Use dense bitmaps for frequent terms.
5. Use compact posting lists for sparse terms.
6. Merge thread-local results efficiently.
7. Preserve sorted document IDs.
8. Avoid duplicate document-term entries.
9. Reuse allocations where possible.

## Important memory considerations

A bitmap for 250,000 documents requires 31.25 KB.

Do not create a full bitmap for every term without calculating total memory.

Consider:

- Document frequency.
- Number of distinct query terms.
- Thread count.
- Thread-local duplication.
- Merge overhead.
- Peak memory during construction.

Choose the sparse/dense threshold based on benchmarks.

## Testing

Test:

- Empty documents.
- Frequent terms.
- Rare terms.
- Repeated terms.
- Duplicate query terms.
- Large posting lists.
- Sorted output IDs.
- Exact document counts and sums.

Compare every query against the baseline implementation.

Benchmark memory and runtime.

Commit:

`perf(track-b): implement memory-efficient indexing`

Acceptance:

- No global pairs vector.
- Correct posting lists.
- Correct bitmap representation.
- Differential tests pass.
- Memory demonstrably reduced.

# PHASE 2 — AVX2 TOKENIZER

Objective:

Reduce the cost of scanning the 254 MB corpus.

The existing scalar tokenizer performs byte-by-byte classification.

Implement AVX2-assisted processing.

## Implementation

1. Process 32-byte blocks.
2. Load unaligned data safely.
3. Identify lowercase letters.
4. Identify uppercase letters.
5. Identify digits.
6. Identify separators.
7. Convert uppercase ASCII to lowercase.
8. Extract token boundaries using bitmasks.
9. Handle tokens spanning block boundaries.
10. Handle the final incomplete block with scalar code.

Use Rust architecture intrinsics.

Check AVX2 support before executing AVX2-specific code unless the official execution contract explicitly guarantees it.

Do not enable unsupported CPU instructions.

## Critical correctness requirements

- UTF-8 bytes remain separators.
- ASCII case conversion only.
- No token truncation.
- Maximum token length respected.
- No boundary errors.
- No incorrect token merging.

Keep the scalar tokenizer as a reference implementation.

## Testing

Run the scalar and SIMD tokenizers against identical test inputs.

Compare their token sequences exactly.

Benchmark tokenizer performance independently.

Commit:

`perf(track-b): optimize tokenizer with AVX2`

Acceptance:

- Exact token equivalence.
- No out-of-bounds memory access.
- SIMD path works on supported hardware.
- Measured performance improvement.

# PHASE 3 — QUERY PREFIX FILTERING

Objective:

Avoid unnecessary hash-table lookups for tokens absent from the query vocabulary.

## Implementation

Build a compact prefix filter from query terms.

Use a 65,536-entry lookup structure indexed by the first two bytes.

For each document token:

1. Calculate its prefix.
2. Check the prefix filter.
3. Reject impossible terms immediately.
4. Perform exact term lookup for possible matches.

Handle one-byte terms separately.

Do not treat prefix matches as actual term matches.

Do not introduce false negatives.

Consider whether a compact bitmap is more efficient than a byte-per-entry table.

## Testing

- Every query term must pass the filter.
- Non-query terms must never create false matches.
- One-byte terms must work.
- Case folding must remain correct.
- Empty and short tokens must be handled correctly.

Benchmark against the original term lookup path.

Commit:

`perf(track-b): add query prefix filtering`

Acceptance:

- No false negatives.
- Exact term verification preserved.
- Memory overhead measured.
- Performance benefit demonstrated.

# PHASE 4 — AHO-CORASICK OPTIMIZATION

Objective:

Reduce phrase-matching transition lookup overhead.

The existing automaton uses hash-based transition lookup.

Replace expensive hot-path operations where beneficial.

## Implementation

1. Inspect automaton state representation.
2. Identify root-state transition frequency.
3. Introduce direct root transition lookup.
4. Store non-root transitions compactly.
5. Consider sorted flat edge arrays.
6. Optimize failure-link traversal.
7. Reduce allocations during automaton construction.
8. Avoid unnecessary state copies.
9. Keep automaton state local to each document.

Do not change phrase semantics.

## Testing

Test:

- Two-token phrases.
- Eight-token phrases.
- Repeated terms.
- Overlapping phrases.
- Punctuation between phrase terms.
- Sentence boundaries.
- Empty documents.
- Document boundary resets.

Compare all phrase results against a reference implementation.

Benchmark AC transitions independently.

Commit:

`perf(track-b): optimize aho-corasick transitions`

Acceptance:

- Exact phrase matching.
- Correct failure links.
- No cross-document matching.
- Improved measured transition performance.

# PHASE 5 — BOOLEAN QUERY ENGINE

Objective:

Improve set operations and query evaluation.

Retain the hybrid sparse-list/dense-bitmap design unless measurements justify a replacement.

## Implementation

Optimize:

- Posting-list intersections.
- Bitmap intersections.
- Bitmap unions.
- Binary set difference.
- Population counts.
- Document ID sums.
- Smallest-ten-ID extraction.

Use word-level bitwise operations where beneficial.

Use hardware POPCNT where supported.

Important:

AVX2 does not provide a general packed 64-bit population-count instruction.

Benchmark scalar hardware POPCNT against other suitable implementations.

All sums must use exact 64-bit arithmetic.

## Query semantics

Boolean queries must remain strictly left-to-right.

Example:

`a OR b AND c`

Means:

`(a OR b) AND c`

NOT is binary set difference.

Never introduce conventional operator precedence.

## Testing

Test:

- AND.
- OR.
- NOT.
- Mixed operators.
- Repeated operands.
- Empty results.
- Full results.
- More than ten matching documents.
- Document ID sums.
- Query ordering.

Differential-test against a simple reference evaluator.

Benchmark each set-operation representation.

Commit:

`perf(track-b): optimize boolean query evaluation`

Acceptance:

- Exact query results.
- Exact counts.
- Exact sums.
- Correct first ten IDs.
- Improved measured performance.

# PHASE 6 — OUTPUT FORMATTING

Objective:

Reduce output allocation and formatting overhead.

## Implementation

1. Allocate a reusable output buffer.
2. Implement efficient integer-to-decimal formatting.
3. Avoid per-query temporary vectors.
4. Write counts and sums.
5. Write up to ten smallest IDs.
6. Preserve exact whitespace.
7. Preserve query order.
8. Write exactly 50,000 lines.
9. Handle zero-match queries as `0 0`.

Do not introduce formatting shortcuts that alter the output.

## Testing

Compare generated output against the official expected result.

Verify:

- Number of lines.
- Whitespace.
- Newlines.
- Integer formatting.
- Counts.
- Sums.
- Document IDs.

Calculate SHA-256.

Commit:

`perf(track-b): optimize output formatting`

Acceptance:

- Exact output.
- Official checksum matches.
- Measured performance improvement or no regression.

---

# 8. DIFFERENTIAL TESTING FRAMEWORK

Create a small, straightforward reference implementation for testing.

The reference implementation must prioritize correctness over speed.

Use generated miniature datasets to compare optimized results against reference results.

Test all supported semantics.

## Mandatory test cases

Tokenization:

- Uppercase ASCII.
- Lowercase ASCII.
- Numbers.
- Alphanumeric tokens.
- Punctuation.
- UTF-8 bytes.
- Empty strings.
- Maximum-length tokens.

Phrase matching:

- Two-token phrase.
- Eight-token phrase.
- Repeated tokens.
- Overlapping phrases.
- Punctuation.
- Sentence boundaries.
- Document boundaries.

Boolean:

- AND.
- OR.
- NOT.
- Mixed operations.
- Strict left-to-right evaluation.
- Lowercase operator-like terms.

Output:

- No matches.
- One match.
- Ten matches.
- More than ten matches.
- Large document ID sums.

Never use the optimized implementation itself as the only correctness oracle.

---

# 9. BENCHMARKING POLICY

Benchmark each phase independently.

Record:

| Metric | Description |
|---|---|
| Wall time | Total process execution time |
| Peak RSS | Maximum resident memory |
| CPU time | Where measurable |
| Checksum | Output SHA-256 |
| Source lines | Counted using competition-compatible method |
| Dependencies | External dependency count |

Use the same corpus and queries for all comparisons.

Perform multiple runs.

Do not compare warm-cache and cold-cache measurements as if they were equivalent.

Document whether the measurement includes:

- Compilation.
- Process startup.
- Input loading.
- Index construction.
- Query evaluation.
- Output writing.

The official benchmark uses one unified execution timer.

Do not optimize a microbenchmark while ignoring total execution time.

Update `PERF.md` after each phase.

Use this format:

| Version | Phase | Runtime | Peak RSS | SHA-256 | Notes |
|---|---|---:|---:|---|---|
| Baseline | Original | 524.5 ms | 311 MB | Verified | Initial |
| Phase 1 | Memory | TBD | TBD | TBD | |
| Phase 2 | Tokenizer | TBD | TBD | TBD | |
| Phase 3 | Prefix | TBD | TBD | TBD | |
| Phase 4 | AC | TBD | TBD | TBD | |
| Phase 5 | Boolean | TBD | TBD | TBD | |
| Phase 6 | Output | TBD | TBD | TBD | |

Never fabricate benchmark values.

---

# 10. FINAL ACCEPTANCE TEST

After all phases:

1. Perform a clean release build.
2. Run all unit tests.
3. Run all differential tests.
4. Run the full official dataset.
5. Verify exactly 50,000 output lines.
6. Verify the official SHA-256.
7. Measure runtime.
8. Measure peak memory.
9. Count source lines using the appropriate competition-compatible method.
10. Confirm zero external dependencies.
11. Inspect all changed files.
12. Inspect the complete Git history.
13. Confirm no unrelated changes are included.
14. Update `PERF.md`.

If any acceptance test fails, do not declare completion.

Create final verification commit:

`test(track-b): verify final correctness and performance`

Do not modify the official checksum to accommodate a failure.

---

# 11. FINAL DELIVERABLES

The completed project must contain:

- Optimized `track-b/src/main.rs`.
- Updated `Cargo.toml`, if required.
- `track-b/IMPLEMENTATION_PLAN.md`.
- Updated `track-b/PERF.md`.
- Correctness tests.
- Differential testing tools.
- Dockerfile or updated Docker configuration.
- Benchmark instructions.
- Final performance report.
- Complete phase-wise Git commit history.

The final report must include:

1. Baseline runtime.
2. Final runtime.
3. Baseline memory.
4. Final memory.
5. Final K.
6. Final source line count.
7. External dependency count.
8. Official checksum verification.
9. All phase commit hashes.
10. Final Git status.
11. Remaining bottlenecks.
12. Whether the leaderboard target was achieved.

---

# 12. EXECUTION ORDER

Follow this exact sequence:

PHASE 0 → PHASE 1 → PHASE 2 → PHASE 3 → PHASE 4 → PHASE 5 → PHASE 6 → FINAL ACCEPTANCE.

For each phase:

- Explain the proposed implementation.
- Inspect the relevant code.
- Identify risks.
- Implement only that phase.
- Compile.
- Test.
- Differential-test.
- Benchmark.
- Update PERF.md.
- Inspect Git diff.
- Commit.
- Verify commit hash.
- Verify Git status.

Only then proceed.

Do not batch all phases into one implementation.

Do not skip benchmarking.

Do not skip commits.

Do not claim success without verification.

---

# 13. START INSTRUCTION

Read this entire document.

Inspect the repository and current Git state.

Do not modify implementation files yet.

Begin with PHASE 0 only.

Report:

- Current branch.
- Existing uncommitted changes.
- Current source architecture.
- Identified bottlenecks.
- Baseline correctness status.
- Baseline checksum status.
- Baseline runtime.
- Baseline peak memory.
- Proposed Phase 1 design.

After Phase 0 passes its acceptance criteria, create its mandatory Git commit.

Wait for confirmation before starting Phase 1.

**The priority is exact correctness, measured performance, low memory consumption, and a clean phase-wise Git history.**