# Track A: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 32.05 | 228 | 0 |
| 1 | Rust rewrite: mmap + `MAP_POPULATE`, one parsing thread per core, no date parsing (hour from the digits), thread-local endpoint interning, counting sort of latencies by (hour, endpoint rank), nearest rank by `select_nth_unstable` | run-fd63abb8fd8e | 0.114 | 61.9 | 61.7 |
| 2 | Drop `MAP_POPULATE` so threads fault their own pages in parallel; find the endpoint's end 8 bytes at a time (SWAR) and hash two loaded words instead of every byte; count per group while parsing, removing a full pass over the records | | | | |

## Notes

- Step 1 profile (8-core laptop, input already in memory): parse 70 ms, scatter 23 ms, select and format 17 ms, everything else under 10 ms each. Parsing dominates, so it gets attention first.
- Step 2 did not speed up parsing on the laptop (still about 70 ms), so the hashing was not the bottleneck there. The board run tells whether the parallel page faults help on Linux.
