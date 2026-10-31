# Track D: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 72.69 | | 0 |
| 1 | Rust rewrite: mmap, two parallel parsing passes (count degrees with atomics, then place edges) into forward and reverse compressed adjacency arrays, bidirectional Dijkstra with hash-table distances, queries spread over all cores | | | | |

## Notes

- Step 1 laptop profile (8 threads, synthetic 1.74M-node lattice with 4.9M edges): read 50 ms, degree counting 115 ms, edge placement 210 ms, 2,000 queries 650 ms. Queries dominate, then the build's random atomic traffic.
