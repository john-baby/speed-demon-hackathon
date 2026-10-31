# Track A: Ingest

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/a-ingest.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`).

1. **Map, don't read.** `/data/requests.log` is `mmap`ed read-only. The host has already loaded it into the page cache, so the 239 MB input is never copied into our memory.
2. **Parse in parallel.** The file is cut into one chunk per available core, at line boundaries. Each thread walks its chunk byte by byte. It never decodes text or parses dates:
   - the hour comes straight from the day and hour digits (the 72-hour span is fixed by the spec, so `hour = day_offset * 24 + HH - 18`);
   - the method is skipped by its first two letters;
   - the endpoint is interned in a small thread-local open-addressing hash table;
   - a status starting with `5` is an error;
   - the latency is parsed as an integer.

   Each line becomes one packed `u64` (endpoint id, 5xx bit, hour, latency).
3. **Rank endpoints once.** The roughly 1,842 distinct endpoints are merged and sorted as bytes. `key = hour * endpoints + rank` is then exactly the output order, so the output is never sorted at the end.
4. **Counting sort into one array.** Every thread counts lines and errors per key. A prefix sum gives each thread its own write offsets, and the threads scatter latencies into one `u32` array grouped by key, in parallel and without locks.
5. **Select, don't sort.** Nearest-rank p50/p95/p99 use three nested `select_nth_unstable` calls on each group's slice, each one searching only above the previous rank. Key ranges with roughly equal line counts are split across threads. Each thread formats its own lines, and the buffers are written out in order.

Peak memory is about 8 bytes per line for the records plus 4 per line for the latency array, so roughly 40 MB in total.

## Dependencies

None.

## With another 24 hours

- Hash and find the endpoint's end in one SWAR pass (8 bytes at a time), not byte by byte.
- Skip the record buffer: parse straight into per-thread per-key counts, then reparse into the latency array (input is in page cache), to cut memory further.
- Profile-guided optimisation with the allowlisted `speed-demon-rust` llvm-tools image.
