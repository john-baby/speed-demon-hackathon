# Track C: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 14.44 | | 0 |
| 1 | Rust rewrite: mmap, parallel quote-parity split into record-aligned chunks, zero-copy parse with SWAR quote scanning, per-thread aggregates, distinct names as group bitmasks merged in parallel hash shards | run-59dd03e83c7f | 0.207 | 102.7 | 47.9 |
| 2 | One shared lock-free name table instead of one per thread (a slot packs a name's file offset, length and hash tag into one word, claimed by compare-and-swap; group bits set with an atomic OR); hash names by length and first and last 8 bytes; count quotes with a byte fold the compiler vectorises; rank country codes through a 26^3 array instead of sorting 200,000 slices | run-94332be51efa | 0.090 | 22.2 | 63.5 |

## Notes

- Step 1 checked against an independent `csv`-module oracle on the spec's worked example, 160 random hostile files (BOMs mid-file, CRLF/LF mix, quoted commas, `""` escapes, multi-line notes, notes starting or ending with a quote, empty quoted and unquoted fields, trailing-space name twins, negative amounts under 1.00, unknown regions) and a 3.32M-record synthetic file.
- Laptop: about 0.77 s on the 3.32M-record file, of which about 0.1 s is Windows reading the file (Linux maps it instead).
- Step 1 profile (laptop, one thread): parsing 1.3 s without name tracking and 2.8 s with it; every thread also kept its own copy of the ~100,000-name pool with group masks (about 7 MB each), which is most of the 103 MB.
- Step 2 on the laptop, 8 threads: regions 140 ms to 15 ms, quote count 90 ms to 15 ms, parse 450 ms to 130-215 ms.
- Step 2 on the board: T 207 ms to 90 ms and M 103 MB to 22 MB.
