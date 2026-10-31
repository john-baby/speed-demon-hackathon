# Track C: Join

The task, the input files and the exact output format are in the [spec](https://github.com/hackathon-raptors/speed-demon-2026/blob/main/specs/c-join.md).

## Approach

Rust, standard library only, one binary (`src/main.rs`).

1. **Regions in parallel.** `regions.csv` is plain, so it is mapped and cut into one range of lines per core. The threads fill `code_of[region_id]`, the region's country code as a number below 26^3, writing disjoint ids. A 26^3 table then gives each code present its rank in sorted order, which is its group. Groups are therefore already in output order, and `_UNMATCHED` (which sorts last) is simply the next index. An id beyond the array, or a hole in it, maps to `_UNMATCHED`.
2. **Split a hostile CSV without tracking quotes.** The spec (section 2.2) guarantees that notes are lowercase words and quotes and that names never hold an LF. So an LF inside a quoted field is never followed by a digit, and an LF followed by a digit or a BOM always ends a record. Each thread starts at an equal byte cut and walks to the first such LF; this holds even when the cut lands inside a multi-line note.
3. **Find fields 64 bytes at a time.** SSE2 compares turn each 64-byte block into masks of its quotes, commas and LFs. A byte is inside quotes when an odd number of quotes precede it, so a running XOR over the quote mask (carried from block to block) marks the quoted bytes, and `""` escapes cancel themselves out. The commas and LFs outside quotes are a record's five delimiters, taken in order from the remaining bits. A field's value length comes from the quotes counted inside it: a field of n bytes holding q quotes is n bytes long if q is 0, else n - 1 - q/2 (two delimiting quotes, and one byte saved per escape). Nothing is copied or unescaped. A BOM is skipped only at the start of a record. Numbers are read without loops: since a field's end is known, one 8-byte load ending there, with the bytes before the field masked off, gives up to 8 digits, which multiplies combine pairwise (the spec bounds ids at 8 digits, region ids at 7 and amounts at 5 before the point). Amounts are exact integer cents.

   Generated non-empty names are always quoted, and quoting is reversible. So the raw bytes between the quotes identify a name's value exactly, and an unquoted empty name and `""` are both an empty slice, the same value.
4. **Distinct names without shared writes.** All threads share one lock-free open-addressing table of names. A 32-byte slot holds a name's file offset and length (claimed by compare-and-swap) and its first 24 bytes, so a lookup is one cache line. Slots are written once, so after the first few thousand records every thread only reads them.
   - Which groups a name occurs in is kept out of the shared table. Group masks (256 bits per name slot) are split into one range per thread, and only the owning thread writes a range.
   - Parsing runs in rounds of about 1 MB per thread. During a round, a thread appends each record's `(name slot, group)` to a small bucket for the owner of that slot. After a barrier, each owner sets the bits it received with plain writes.
   - An earlier version set the bits with an atomic OR in the shared slot, which invalidated the line other threads were reading the key from. A board profile put that at about 22 ms of a 59 ms run.
   - Each record is finished one record later. Its name slot and its entry in the 2 MB region-to-country array are prefetched as soon as they are known, and used while the next record is parsed. A name's key is three word loads masked to its length, not a copy. The scanner state lives in locals and the finishing step is inlined, so the hot loop keeps its state in registers rather than copying a record tuple through memory.
   - `names_distinct` for a group is the number of masks with its bit set, counted by each owner over its range. Count, id sum, amount sum, min and max, and byte totals are plain per-thread counters, summed at the end.

## Dependencies

None.

## With another 24 hours

- A board profile shows each thread spends about 45 ms parsing its 17 MB, about 160 ns per record (the box runs without turbo), while the rounds, owners and barriers cost about 4 ms. The next step is a parser with about half the instructions per record. A first attempt (AVX2 masks, delimiters scanned ahead, loop-free number parsing) measured no faster, so the cost is elsewhere, likely in the name probe's unpredictable loop and the per-record bookkeeping.
- A SIMD tag array beside the name table (one byte per slot, 16 compared at once) would make most lookups a single slot read with no probing loop.
