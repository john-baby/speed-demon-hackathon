# Track C: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 14.44 | | 0 |
| 1 | Rust rewrite: mmap, parallel quote-parity split into record-aligned chunks, zero-copy parse with SWAR quote scanning, per-thread aggregates, distinct names as group bitmasks merged in parallel hash shards | | | | |

## Notes

- Step 1 checked against an independent `csv`-module oracle on the spec's worked example, 160 random hostile files (BOMs mid-file, CRLF/LF mix, quoted commas, `""` escapes, multi-line notes, notes starting or ending with a quote, empty quoted and unquoted fields, trailing-space name twins, negative amounts under 1.00, unknown regions) and a 3.32M-record synthetic file.
- Laptop: about 0.77 s on the 3.32M-record file, of which about 0.1 s is Windows reading the file (Linux maps it instead).
