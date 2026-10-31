# Track D: Traverse

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/d-traverse.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`). Every phase runs on all cores. [PERF.md](PERF.md) logs each step with the board run that measured it.

1. **Map, don't read.** `graph.txt` is `mmap`ed read-only and cut into one chunk per core at line boundaries.
2. **Parse once, build without atomics.** Each thread parses its chunk once and sends every edge to the bucket of its tail's part. The id range is split into eight parts per thread. Numbers are read without a loop per digit: separators are the bytes without bit 4 set, so one 8-byte load gives a number's length, and three multiplies combine its digits. Each thread then owns eight consecutive parts. It counts its nodes' out-degrees and places their edges (`[head, weight]`, 8 bytes) with plain writes. Because a part's edges land in a region of about 400 KB, the scattered writes stay in cache. Self-loops are dropped, since they never shorten a path. Parallel edges and duplicate lines are kept, and Dijkstra uses whichever is lightest.
3. **Renumber for locality while ordering.** Node ids are random, so without renumbering a search jumps across the whole graph on every step.
   - Every thread grows breadth-first regions from seeds in its own slice of the ids. The threads' visiting orders, one after another, become the new ids.
   - A thread claims a node by compare-and-swapping its provisional id (thread, place in that thread's order) into one atomic array. Reading a neighbour's slot therefore gives either nothing, in which case the reader claims it, or its id.
   - Each thread writes a visited node's edges, renamed provisionally, while the BFS is reading them anyway. A final parallel pass maps provisional ids to final ones while copying the edges into place, and counts in-degrees. Each thread then frees its own part, so that memory is given back in parallel. The claim array and the in-degree counters come from calloc, so the threads fault their zero pages in as they first write them, instead of one thread filling 7 MB up front.
   - The old graph is read once, with software prefetches at three depths (offsets, edges, heads' slots) to keep several cache misses in flight.
   - The reverse graph is the transpose of the renumbered forward graph. Since an edge's two ends now have nearby ids, its counters and writes stay local. The in-degrees become running totals (each node's end) in a parallel prefix sum. Each edge is then placed by counting its head down, which leaves the counters at the starts, so they serve as the offsets without a copy.
4. **Bidirectional Dijkstra per query.** A forward search runs from `s` and a backward search from `t`, and each step expands the side with the smaller queue. A side about to run dry (an island, a dead end, a target with no in-edges) therefore finishes after a handful of nodes, instead of letting the other side flood the graph. A relaxation that reaches a node the other side has reached gives a candidate path. The search stops once the two frontier keys together reach the best candidate.
5. **Cheap search state.**
   - Distances live in a paged array. Renumbered searches touch few blocks of 1,024 consecutive ids, and only those get a page. A lookup is two array reads, and clearing costs only what the search touched. After a search that needed more than 64 pages, the extra pages are freed. glibc's mmap threshold is fixed at 256 KB, so freed pages go back to the system instead of staying with the allocator, and 24 maps do not each keep the largest search they ever ran.
   - Distances are stored as `u32` and heap entries are one `u64` (`distance << 21 | node`). A search whose distances reach 2^32 - 1 (the spur and no-in-edge queries) is rerun with `u64` distances and pair entries, so answers above 2^32 stay exact.
   - After each pop, the edges of the node left on top of the heap are prefetched, since it is usually the next one settled. A node's offsets are prefetched when it is queued, so that this prefetch does not wait on them.
   - The stop rule reads the heap tops as they are. A stale entry only lowers a top, so the rule stays safe, and stale entries (2% of pops) are skipped when popped instead of being looked up on every peek.
6. **Parallel queries.** Every core takes the next unanswered query from an atomic counter. Answers are written in input order at the end.

## Dependencies

None.

## With another 24 hours

- Place the reverse graph in the same pass that fixes up the forward one, using in-degrees counted on the old ids, to save the transpose's pass over the edges.
- Shrink the graphs, which are most of the memory (two copies of 5M 8-byte edges), for example by storing small weights in fewer bits.
- A goal-directed search (landmarks) would settle far fewer nodes per query. Its preprocessing (full searches from each landmark) costs more than the 2,000 queries it would save at this scale, but would pay off with more queries.
