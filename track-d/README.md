# Track D: Traverse

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/d-traverse.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`).

1. **Map, don't read.** `graph.txt` is `mmap`ed read-only and cut into one chunk per core at line boundaries.
2. **Build both graphs without an edge buffer.** The first parallel pass parses every line and counts out- and in-degrees with atomic counters. A prefix sum turns the counts into offsets. The second pass parses again and drops each edge straight into its slot in a forward and a reverse compressed adjacency array (`[head, weight]` pairs, 8 bytes per edge per direction). Self-loops are skipped, since they never shorten a path. Parallel edges and duplicate lines are kept; Dijkstra uses whichever is lightest.
3. **Bidirectional Dijkstra per query.** A forward search runs from `s` over the forward graph and a backward search from `t` over the reverse graph. Each step expands whichever side has the smaller queue, so a side about to run dry (an island, a dead end) finishes first instead of letting the other side flood the graph. Whenever an edge relaxation reaches a node the other side has already reached, the sum is a candidate path length. The search stops once the two frontier keys together reach the best candidate. If either side's queue empties, nothing better can appear, so an unreachable target (one with no in-edges, a separate island) is found to be unreachable after touching only a handful of nodes. Distances are `u64`, because paths can exceed 2^32.
4. **Small per-search state.** Tentative distances live in an open-addressing hash table, cleared by walking the slots it used. A search therefore costs what it touches, and memory does not grow with the number of threads times the number of nodes.
5. **Locality by renumbering.** Node ids are random, so a search jumps across 80 MB of adjacency on every step. After loading, every thread grows breadth-first regions from seed nodes in its own slice of the id range, claiming each node with an atomic flag. The threads' visiting orders, concatenated, become the new ids, and both graphs are rebuilt in parallel in that order. Nearby nodes then share cache lines and pages, which halves query time. Query endpoints are mapped through the same table.
6. **Parallel queries.** Every core takes the next unanswered query from an atomic counter, and the answers are written in input order at the end.

## Dependencies

None.

## With another 24 hours

- Fuse the renumbering into the build, so the edges are placed once, directly under their new ids.
- Replace the binary heap with a radix heap.
- Cut the atomic traffic in the build, for example with per-thread partial counts over node ranges.
