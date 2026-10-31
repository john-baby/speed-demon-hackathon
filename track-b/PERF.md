# Track B: performance log

Every step from the reference to your final version, each with the live-board run ID that measured it. Judges check that the run IDs exist and that each change moved the way you say it did.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 56.35 | | 0 |
| 1 | Rust rewrite: index only the query terms in one parallel mmap scan; all phrases matched during the scan by an Aho-Corasick automaton over term ids; BOOL posting lists by parallel counting sort; hybrid sorted-list / bitmap evaluation strictly left to right | run-9ab4fffdc3ce | 0.679 | 313.1 | 50.2 |

## Notes

- Before step 1 was pushed, the BOOL answers took 33 s on a synthetic full-size corpus: intersecting two 250,000-document lists by binary search costs 20-30 ms, and chains like `the AND in AND of` repeat it. Bitmaps for every term in more than N/32 documents brought the answer phase to 0.35 s.
- Step 1 laptop profile (8 threads, synthetic 250,000 documents, 50,000 queries): read 150 ms (Windows), scan 600 ms, postings 60 ms, phrase hits 100 ms, answers 350 ms.
- Checked against a reimplementation of the reference on the spec's worked example and 240 random corpora (upper case, non-ASCII separators, punctuation joins, repeated runs, over-long tokens, empty documents, left-to-right operator chains, scrambled and repeated phrases, absent terms).
- Step 1 was re-run as a side effect of a later push (run-0ac3ba26b183): 0.590 s, 310.7 MB, 51.8 points, same code; the board's spread between runs of identical code is visible here.
