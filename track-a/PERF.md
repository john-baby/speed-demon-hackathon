# Track A: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 32.05 | 228 | 0 |
| 1 | Rust rewrite: mmap + `MAP_POPULATE`, one parsing thread per core, no date parsing (hour from the digits), thread-local endpoint interning, counting sort of latencies by (hour, endpoint rank), nearest rank by `select_nth_unstable` | run-fd63abb8fd8e | 0.114 | 61.9 | 61.7 |
| 2 | Drop `MAP_POPULATE` so threads fault their own pages in parallel; find the endpoint's end 8 bytes at a time (SWAR) and hash two loaded words instead of every byte; count per group while parsing, removing a full pass over the records | run-9dda0d629944 | 0.097 | 65.3 | 63.2 |
| 3 | Two passes over the mapped input instead of a 25 MB record buffer: pass 1 interns endpoints and counts, keeping only a 2-byte endpoint id per line; pass 2 rereads each line's hour and latency (the latency backwards from the line end) and scatters it straight into its slot | run-a2814457b11a | 0.092 | 43.5 | 65.5 |
| 4 | Cheaper per line: skip the method with one word read; compare endpoints by stored first/last word and length (memcmp only for the middle of names over 16 bytes); in pass 2 start the newline search past the known endpoint length; parse the latency with SWAR (find the last space by bit 4, three multiplies for up to 8 digits); store latencies endpoint-major so building the offsets walks each thread's tally in order | run-0e98f12c25d3 | 0.069 | 51.7 | 67.6 |
| 5 | One pass over the input: SIMD masks mark every space and newline 64 bytes at a time, and since a line has exactly four spaces and a newline the separators come in fives and no field is searched for; the endpoint is checked with one fixed-size 64-byte compare (no `memcmp`); each latency goes straight into its (hour, endpoint) chain of 8-entry blocks in the thread's own arena, with no per-line ids, no counting pass, no rereading and no global latency array; names are merged by hash and sorted as copies, not as slices of the input; hand-written number formatting; the main thread writes finished hours while the workers select | run-c41e36ab5c7e | 0.040 | 47.6 | 73.4 |
| 6 | Group table as per-hour rows grown on demand instead of a fixed [72][4096] table per thread, which the select phase's probing of every hour touched in full (about 3.5 MB per thread); workers write their own hour as soon as the previous one is written, reusing one buffer, instead of queueing finished hours to the main thread | run-5d68ce8c3fe8 | 0.037 | 38.9 | 75.0 |

## Notes

- Step 1 profile (8-core laptop, input already in memory): parse 70 ms, scatter 23 ms, select and format 17 ms, everything else under 10 ms each. Parsing dominates, so it gets attention first.
- Step 2 did not speed up parsing on the laptop (still about 70 ms), so the hashing was not the bottleneck there. The board run tells whether the parallel page faults help on Linux.
- Step 2 on the board: 114 ms to 97 ms, so on Linux the parallel page faults (no `MAP_POPULATE`) were worth about 17 ms. Memory did not move (the record buffer is still there), which motivates step 3.
- Single-thread laptop profile before step 3: pass 1 (parse and count) about 72 ns per line, scatter about 36 ns per line.
- Step 3 on the board: memory 65.3 MB to 43.5 MB as the record buffer went, and T 97 ms to 92 ms.
- Before step 4, single-thread laptop profile: pass 2 dropped from about 110 ms to 80 ms with the SWAR latency; building the offsets in output (hour-major) order missed cache on every thread's tally.
- Step 4 on the board: T 92 ms to 69 ms. M rose from 43.5 MB to 51.7 MB, which the change does not explain by itself (the endpoint table grew by under 2 MB); M is the highest of five runs, so the next run will show whether it is noise.
- Before step 5, an 8-thread laptop profile on a time-ordered 3.1M-line file: pass 1 46 ms, offsets 12 ms, pass 2 21 ms, select 15 ms, write 6 ms. About 8 ms of the offsets phase was the endpoint sort, whose comparisons read slices of the 240 MB input and missed cache nearly every time. Verifying endpoints with `memcmp` cost about 24 ns per line.
- After step 5, the same profile: parse 28 ms, rank 1.3 ms, select and write 16.5 ms. The laptop total went from 101 ms to 46 ms (the minimum of 10 interleaved runs), and the output is byte-identical on the test suite, a shuffled 3.1M-line file and a time-ordered one.
- Step 5 on the board: T 69 ms to 40 ms. M stayed near 48 MB, against an estimate of 22-25 MB from the arena (about 15 MB) and tables, so the rest of the memory still needs finding.
- Before step 6: at 12 threads on the laptop, the select phase grew the working set by about 21 MB while committed memory grew by only 2 MB. It was touching existing memory, not allocating: the probes `groups[h][id]` read every hour's slice of every thread's table. The writer queue held up to about 6 MB more when the main thread fell behind.
- After step 6, on the laptop: peak working set beyond the input buffer about 50 MB to 32 MB at 12 threads, and 46 MB to 29 MB at 8 threads; time unchanged within noise. On the board: M 47.6 MB to 38.9 MB, T 40 ms to 37 ms.
