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
| 6 | Back to the binary heap. A distance table that a long search grew past 2^17 slots is replaced by a small one after the query, so twelve threads do not each hold the largest table they ever needed; the renumbering table is freed once the query endpoints are mapped | run-24f858881752 | 0.311 | 152.6 | 59.5 |
| 7 | Distance tables start at 2^12 slots instead of 2^16 and are shrunk back after any query that grew them past 2^15: 24 tables at 2^16 were 19 MB before any search grew them | run-26daa34fc68b | 0.321 | 152.9 | 59.1 |
| 8 | Distances in a paged array instead of a hash table: node ids are renumbered for locality, so a search touches few blocks of 1,024 consecutive ids, and only those get a page (a directory per side maps blocks to pages); a lookup is two array reads with no probing | run-578ac8f5e4d5 | 0.279 | 174.4 | 60.4 |
| 9 | Heap entries packed into one u64 (distance << 21 \| node) instead of a (u64, u32) pair: half the size and one compare; a search whose distances outgrow the packing is run again with pairs | run-4c5473b3a427 | 0.259 | 183.4 | 60.7 |
| 10 | Parse `graph.txt` once instead of twice: each thread sends every edge to a bucket for the thread that owns its tail's id range, then each owner counts and places its own nodes' edges with plain writes into a range that fits its cache, instead of atomic counters and writes spread over the whole graph | run-dd0f5eb4efca | 0.236 | 165.7 | 61.8 |
| 11 | Distances stored as u32 in the common search (u64 only in the fallback search, now taken at 2^32 - 1), halving the pages a search touches | run-a9c3305e1585 | 0.227 | 136.9 | 62.9 |
| 12 | In-degrees for the transpose counted while renumbering (the renamed head is already in hand), so the transpose makes one pass instead of two; the rank table and the renumbered offsets built in parallel, each thread over its own BFS order, instead of two serial loops over 1.74M nodes | run-eafebba5e167 | 0.216 | 131.2 | 63.4 |
| 13 | Each owner's id range split into 8 parts, bucketed separately while parsing, so placing a part's edges writes into a region of about 400 KB that stays in cache instead of scattering over the owner's 3 MB; software prefetches in renumbering (offsets 16 nodes ahead, edges 8 ahead, heads' ranks 4 ahead) and in the BFS | run-e2488bd14f73 | 0.197 | 131.8 | 64.1 |
| 14 | Graph numbers parsed without a loop per digit: separators are the bytes without bit 4 set, so one 8-byte load gives a number's length, and its digits are combined in pairs with three multiplies | run-94686787028e | 0.188 | 131.7 | 64.6 |

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
- Step 6 on the board: memory did not drop (153 MB), so the long-query tables were not the peak; the peak is the query phase itself, where both graphs (94 MB) sit next to 24 distance tables that start at 2^16 slots (12 bytes each).
- Steps 6 and 7 on the board: M stayed at about 153 MB with smaller distance tables, so they are not the peak either; steps 4 to 7 sit within the board's run-to-run spread (T 0.30-0.32 s, 59.1-60.0 points). Finding the rest of the memory needs a Linux profile, which this laptop cannot give.
- Board profile of step 7 (run-fed5fa868fe6, an instrumented build that writes its phase times into an extra file name, so the run fails the output check on purpose): count 30 ms, place 40 ms, ordering 29 ms, renumber 25 ms, transpose 21 ms, queries 170 ms. Steps 8 to 11 go after the queries and the double parse.
- Step 8 on the laptop: queries 390 ms to 220 ms. A query settled about 4,800 nodes on average (9.57M over 2,000 queries) at about 220 ns each, too slow for a search that fits in cache, which pointed at the hash probes and the heap.
- Tried and dropped after step 9: a 4-ary heap of u64 keys; queries 210 ms to 300 ms on the laptop. The standard binary heap's sift-to-the-bottom pop is hard to beat.
- Step 10 on the laptop: parse and forward build 205 ms to 145 ms.
- Page size: with 1,024-id pages a search uses at most 259 pages (2 MB) and 50 on average; 256-id pages made no difference to time or peak memory.
- Step 11 on the laptop: queries 189 ms to 160 ms; on the board M fell 166 MB to 137 MB.
- Board profile of step 12 (run-62e9ad7684ff, instrumented, fails the output check on purpose): parse into buckets 24.0 ms, owners place edges 23.6 ms, BFS ordering 19.9 ms, renumbering 27.8 ms, transpose 12.3 ms, queries 94.3 ms. The queries settle 9.57M nodes with 10.4M heap pushes and only 0.18M stale pops on the synthetic graph, so the search itself wastes little; the build had more to give.
- Before step 13, one thread on the laptop: parse 125 ms, count 40 ms, place 145 ms, BFS 160 ms, renumber 265 ms, transpose 60 ms; placing and renumbering are random-access bound. With 8 threads, step 13 took placing from about 56 ms to 18 ms, renumbering from 83 ms to 65 ms and the BFS from 40 ms to 34 ms.
- Step 14 on the laptop: the parse took a third less time on one thread.
