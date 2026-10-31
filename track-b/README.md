# Track B: Index

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/b-index.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`). The queries are known before the corpus is read, so the program indexes only what they ask about, in one parallel pass.

1. **Queries first.** Every distinct query term gets an id in a small open-addressing table, keyed by the term zero-padded to 32 bytes. Each term is flagged as used in a BOOL query, in a PHRASE query, or both.
2. **One automaton for all phrases.** The 19,000 phrases are inserted as paths of term ids into an Aho-Corasick automaton, with failure links and "nearest phrase end" output links. A phrase then matches wherever the automaton reaches its end node. A token that is in no phrase resets the automaton to the root, because no phrase can span it. This needs no positional index and no second look at any document. It also handles phrases made only of very common words (`"the the"`), which would make any candidate-then-verify approach scan most of the corpus.
3. **One parallel scan.** `docs.txt` is `mmap`ed and cut into one range of whole lines per core. Each thread tokenizes byte by byte with a 256-entry table, following the spec's rule exactly:
   - `A`–`Z` are lowercased; `a`–`z` and `0`–`9` are token bytes;
   - everything else, including every byte ≥ 0x80, is a separator;
   - tokens over 32 bytes cannot be query terms.

   Each token's term id feeds two things. If the term is a BOOL term, a (term, doc) pair is recorded, once per document. If it is a phrase term, the automaton advances, and every phrase end it reaches records a (phrase, doc) hit, once per document.
4. **Posting lists by counting sort.** The per-thread counts become offsets into one `u32` array. The threads scatter their pairs in parallel, and because each thread holds an ascending range of documents, every list comes out sorted.
5. **Hybrid sets for BOOL.** Any term in more than N/32 documents also gets a bitmap over all documents (31 KB at 250,000 documents). The running result of a query, applied strictly left to right, is a sorted list while it is small and a bitmap once it grows:
   - AND or NOT against a bitmap term filters the list with one bit test per element, or works word by word on two bitmaps;
   - AND against a short list keeps only that list's elements whose bit is set in the running bitmap;
   - an OR that would grow past the threshold switches to a bitmap.

   No operation costs more than the size of its smaller side, or about 4,000 word operations.
6. **Output.** For each query it writes the count, the sum and the ten smallest ids, computed in one pass over the list or the set bits. Queries are answered on every core and written in input order.

## Dependencies

None.

## With another 24 hours

- Speed up the scan, which now dominates: hash tokens incrementally instead of building a zero-padded key per token, and skip the term-table lookup for tokens that cannot be query terms.
- Store postings for very frequent terms only as bitmaps, to cut memory.
