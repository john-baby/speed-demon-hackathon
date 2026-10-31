# Track A: Ingest

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/a-ingest.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`), x86-64 (SSE2 intrinsics).

1. **Map, don't read.** `/data/requests.log` is `mmap`ed read-only. The host has already loaded it into the page cache, so the 239 MB input is never copied into our memory.
2. **One pass, in parallel.** The file is cut into one chunk per available core, at line boundaries. Each thread reads its chunk exactly once and never decodes text or parses dates:
   - SIMD compares mark every space and newline, 64 bytes at a time. A line has exactly four spaces and a newline, so the separator positions come in fives and give every field without searching;
   - the hour comes straight from the day and hour digits (the 72-hour span is fixed by the spec, so `hour = day_offset * 24 + HH - 18`);
   - the endpoint is interned in a small thread-local hash table and checked against the stored name with one fixed-size 64-byte compare;
   - a status starting with `5` is an error; the latency is parsed with a few multiplies on one 8-byte word.

   Each latency is appended to its (hour, endpoint) group in a chain of 8-entry blocks in the thread's own arena, so nothing is counted first and nothing is copied later. The input's lines are roughly in time order, so a chunk touches only a few hours' groups.
3. **Rank endpoints once.** The 1,842 distinct endpoints from all threads are merged and sorted as bytes; each thread's ids are mapped to that rank, which is the output order.
4. **Select, don't sort.** Threads take hours one at a time. For each group they gather its blocks from every chunk and find nearest-rank p50/p95/p99 with three nested `select_nth_unstable` calls, each searching only above the previous rank. The main thread writes finished hours out in order while the others work.

Peak memory is the latencies (4 bytes per line) plus a few bytes per group of slack and chain links: about 15 MB for the full input, plus small per-thread tables.

## Dependencies

None.

## With another 24 hours

- Profile-guided optimisation with the allowlisted `speed-demon-rust` llvm-tools image.
- A histogram-based selection for the largest groups instead of `select_nth_unstable`.
