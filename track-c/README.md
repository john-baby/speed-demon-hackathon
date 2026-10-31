# Track C: Join

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/c-join.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`).

1. **Regions in parallel.** `regions.csv` is plain, so it is mapped and cut into one range of lines per core. The threads fill `code_of[region_id]`, the region's country code as a number below 26^3, writing disjoint ids. A 26^3 table then gives each code present its rank in sorted order, which is its group. Groups are therefore already in output order, and `_UNMATCHED` (which sorts last) is simply the next index. An id beyond the array, or a hole in it, maps to `_UNMATCHED`.
2. **Split a hostile CSV without tracking quotes.** The spec (section 2.2) guarantees that notes are lowercase words and quotes and that names never hold an LF. So an LF inside a quoted field is never followed by a digit, and an LF followed by a digit or a BOM always ends a record. Each thread starts at an equal byte cut and walks to the first such LF; this holds even when the cut lands inside a multi-line note.
3. **Parse without copying.** Each thread walks its records:
   - a BOM is skipped only at the start of a record;
   - integers and amounts are read as exact integer cents;
   - quoted fields are scanned eight bytes at a time for the next `"`, counting `""` escapes rather than unescaping them.

   Generated non-empty names are always quoted, and quoting is reversible. So the raw bytes between the quotes identify a name's value exactly. An unquoted empty name and `""` both give an empty slice, so they are the same value. A value's byte length is its raw length minus its escapes.
4. **Distinct names without per-country sets.** All threads share one lock-free open-addressing table of 64-byte, cache-line-aligned slots. A slot is claimed by compare-and-swap and holds the name's file offset and length, its first 24 bytes and a 256-bit mask of the groups the name occurs in. A lookup compares the inline bytes (and, for the rare name over 24 bytes, the rest against its first occurrence in the file), then sets the group's bit with an atomic OR, so it costs one cache miss. The table is 8 MB at 2^17 slots, under 80% full for the generator's pool of 100,000 names. `names_distinct` for a group is the number of masks with its bit set. Count, id sum, amount sum, min and max, and byte totals are plain per-thread counters, summed at the end.

## Dependencies

None.

## With another 24 hours

- Profile on Linux and tune the hot loop (field scanning and the name hash).
- Fuse the quote-counting pass into the parse for all but the first record of each chunk.
