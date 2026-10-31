# Track D: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 72.69 | | 0 |
| 1 | Rust rewrite: mmap, two parallel parsing passes (count degrees with atomics, then place edges) into forward and reverse compressed adjacency arrays, bidirectional Dijkstra with hash-table distances, queries spread over all cores | run-9439da7f8c89 | 1.431 | 461.9 | 39.0 |
| 2 | Alternate the two searches by smaller queue instead of smaller key: an island source's forward search keeps near-2^32 spur nodes queued, so the key rule let the backward search flood the whole 1.74M-node main body before the island ran dry | | | | |

## Notes

- Step 1 laptop profile (8 threads, synthetic 1.74M-node lattice with 4.9M edges): read 50 ms, degree counting 115 ms, edge placement 210 ms, 2,000 queries 650 ms. Queries dominate, then the build's random atomic traffic.
- Step 1 on the board used 462 MB where the laptop used about 120 MB. My synthetic graph has no islands; on the real one, the four "unreachable, island" queries each settled the whole main body, and their distance tables grew to match.
- Tried and parked: renumbering nodes in BFS order halved query time on the laptop (650 ms to 350 ms) but the single-threaded BFS (300 ms) and rebuild (150 ms) cost more than it saved.
