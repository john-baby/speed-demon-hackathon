# Track D: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 72.69 | | 0 |
| 1 | Rust rewrite: mmap, two parallel parsing passes (count degrees with atomics, then place edges) into forward and reverse compressed adjacency arrays, bidirectional Dijkstra with hash-table distances, queries spread over all cores | run-9439da7f8c89 | 1.431 | 461.9 | 39.0 |
| 2 | Alternate the two searches by smaller queue instead of smaller key: an island source's forward search keeps near-2^32 spur nodes queued, so the key rule let the backward search flood the whole 1.74M-node main body before the island ran dry | run-2e262b3e7e3a | 0.446 | 132.9 | 57.1 |
| 3 | Renumber nodes for locality, in parallel: each thread grows breadth-first regions from seeds in its own id range, claiming nodes with an atomic flag; the threads' visiting orders, concatenated, are the new ids, and both graphs are rebuilt in parallel in that order | run-ec6a476241db | 0.384 | 156.4 | 57.3 |
| 4 | Build only the forward graph from the text (one atomic counter per edge instead of two, half the random writes), renumber it, then derive the reverse graph by transposing the renumbered forward graph, where an edge's ends are mostly close so the counters and writes stay local | run-34c1a43625fb | 0.304 | 145.4 | 60.0 |
| 5 | Radix heap instead of a binary heap: Dijkstra only pushes keys at or above the last minimum, so an entry can sit in the bucket of the highest bit where it differs from that minimum (O(1) push, few moves per entry). Reverted: 3% faster, but the extra code (S 390 to 434) and buckets (M +10 MB) cost more than it gained | run-2c55876175b1 | 0.295 | 155.2 | 59.7 |
| 6 | Back to the binary heap. A distance table that a long search grew past 2^17 slots is replaced by a small one after the query, so twelve threads do not each hold the largest table they ever needed; the renumbering table is freed once the query endpoints are mapped | | | | |

## Notes

- Step 1 laptop profile (8 threads, synthetic 1.74M-node lattice with 4.9M edges): read 50 ms, degree counting 115 ms, edge placement 210 ms, 2,000 queries 650 ms. Queries dominate, then the build's random atomic traffic.
- Step 1 on the board used 462 MB where the laptop used about 120 MB. My synthetic graph has no islands; on the real one, the four "unreachable, island" queries each settled the whole main body, and their distance tables grew to match.
- Tried and parked: renumbering nodes in BFS order halved query time on the laptop (650 ms to 350 ms) but the single-threaded BFS (300 ms over both directions, 180 ms over forward edges only) and the rebuild of both graphs (140 ms) cost as much as it saved.
- Tried and dropped: one hash table holding both directions' distances (one probe per relaxation instead of three). Query time did not move (650 ms), so the probes are not the bottleneck; the random reads into the 80 MB graph are.
- Step 2 on the board: T 1.43 s to 0.446 s and M 462 MB to 133 MB, confirming the island queries were the outlier.
- Step 3 on the laptop: the parallel ordering takes 50 ms instead of 180-300 ms single-threaded, the rebuild 130-160 ms, and queries drop from 650 ms to about 330 ms; the whole run goes from 1.1 s to 0.85-0.95 s.
- Step 3 on the board: T 446 ms to 384 ms, but M 133 MB to 156 MB because both graphs existed twice during the rebuild, so points barely moved. Step 4 removes the second copy of the reverse graph.
- Step 4 on the laptop: count 45 ms, place 105 ms, ordering 50 ms, renumber 65 ms, transpose 35 ms, queries 350 ms; 0.7 s in all against 0.85-0.95 s for step 3.
- Step 4 on the board: T 384 ms to 304 ms.
- Tried and dropped before step 5: indexing the distance table by the node id itself (consecutive ids form clusters that linear probing walks through, slower) and by blocks of 8 ids (no change); the probes are not the bottleneck.
- Step 5 showed no change on the laptop (queries 330-370 ms either way), so it goes to the board to be measured.
- Tried before step 6: choosing the side by smaller frontier key settles 9.71M nodes on the synthetic graph, the queue-size rule 9.57M, so the queue-size rule stays.
