# Track A: Ingest

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/a-ingest.md).

Submitted version: 0.036 s, 39 MB, 75.5 points on the live board (run-f5b63b76f24a), against 32.05 s for the reference. [PERF.md](PERF.md) logs every step with the run that measured it.

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
4. **Select, don't sort.** Threads take hours one at a time. For each group they gather its blocks from every chunk and find nearest-rank p50/p95/p99 with three nested `select_nth_unstable` calls, each searching only above the previous rank. Each thread writes its hour as soon as the hour before it is written, so only one hour of output per thread is ever held. Once the last hour is written the process exits at once, without freeing its arenas and tables one by one.

Our own peak memory is the latencies (4 bytes per line) plus a few bytes per group of slack and chain links, about 15 MB for the full input, plus small per-thread tables: about 27 MB in all. The board reports about 39 MB, and the difference matches the 10 MB output file, whose page cache appears to be charged to the run.

## Dependencies

None.

## With another 24 hours

- A board profile shows the parse threads taking anywhere from 12 to 26 ms on the same amount of input, which suggests uneven cores. Work stealing and other thread counts did not help, so the next step is to find out why some cores are slower.
- Keep only a 16-bit code per latency, and collect exact values in a second pass only for the buckets holding each percentile. That would cut memory to a few MB, but costs a second parse.
- Profile-guided optimisation with the allowlisted `speed-demon-rust` llvm-tools image.
