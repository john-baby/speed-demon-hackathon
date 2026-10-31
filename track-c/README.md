# Track C: Join

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/c-join.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`).

1. **Regions first.** `regions.csv` is plain, so it is parsed straight into an array indexed by `region_id`. Each slot holds the region's group, which is the rank of its country code in sorted order. Groups are therefore already in output order, and `_UNMATCHED` (which sorts last) is simply the next index. An id outside the array, or a hole in it, maps to `_UNMATCHED`.
2. **Split a hostile CSV safely.** Each quoted field contributes an even number of `"` bytes (two delimiters plus `""` escapes in pairs). So a byte is inside quotes exactly when an odd number of `"` bytes come before it. One parallel pass counts the quotes in each of N equal byte ranges. The running parity then says, for each cut, whether it falls inside a quoted field. From each cut, a short walk finds the first LF outside quotes, and the next record starts there. This holds even when a cut lands inside a multi-line note.
3. **Parse without copying.** Each thread walks its records:
   - a BOM is skipped only at the start of a record;
   - integers and amounts are read as exact integer cents;
   - quoted fields are scanned eight bytes at a time for the next `"`, counting `""` escapes rather than unescaping them.

   Generated non-empty names are always quoted, and quoting is reversible. So the raw bytes between the quotes identify a name's value exactly. An unquoted empty name and `""` both give an empty slice, so they are the same value. A value's byte length is its raw length minus its escapes.
4. **Distinct names without per-country sets.** Each thread interns names in its own hash table, storing a bitmask of the groups each name occurred in. The threads then merge these tables in parallel shards by name hash, OR-ing the masks, and count each group's bit. Count, id sum, amount sum, min and max, and byte totals are plain per-thread counters, summed at the end.

## Dependencies

None.

## With another 24 hours

- Profile on Linux and tune the hot loop (field scanning and the name hash).
- Fuse the quote-counting pass into the parse for all but the first record of each chunk.
