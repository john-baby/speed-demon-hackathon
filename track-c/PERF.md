# Track C: performance log

Every step from the reference to the submitted version, each with the live-board run that measured it. Reverted experiments are listed too, with why they were dropped.

**Summary.** 14.44 s for the reference, then 0.207 s for the first Rust version, and **0.048 s / 17 MB / 70.2 points** for the submitted step 11 (run-87949b19dc88). The big moves:
- step 2: one shared lock-free name table;
- step 4: record-boundary splitting from the spec's guarantees, instead of a quote-counting pass;
- steps 5 and 7: prefetching the name slot and region entry one record ahead;
- step 8: SIMD structural scanning, with group masks owned per thread so no cache line is written by two cores;
- step 11: a register-resident record loop with loop-free numbers.

Board profiles (steps 7, 10 and 11) guided each change.

| Step | Change | Run ID | T (s) | M (MB) | Points |
|---|---|---|---|---|---|
| 0 | The reference, unchanged | | 14.44 | | 0 |
| 1 | Rust rewrite: mmap, parallel quote-parity split into record-aligned chunks, zero-copy parse with SWAR quote scanning, per-thread aggregates, distinct names as group bitmasks merged in parallel hash shards | run-59dd03e83c7f | 0.207 | 102.7 | 47.9 |
| 2 | One shared lock-free name table instead of one per thread (a slot packs a name's file offset, length and hash tag into one word, claimed by compare-and-swap; group bits set with an atomic OR); hash names by length and first and last 8 bytes; count quotes with a byte fold the compiler vectorises; rank country codes through a 26^3 array instead of sorting 200,000 slices | run-94332be51efa | 0.090 | 22.2 | 63.5 |
| 3 | One 64-byte cache line per name slot holding the claimed offset and length, the name's first 24 bytes and its group mask, so a lookup is one cache miss instead of three; names over 24 bytes compare the rest against their first occurrence; 2^17 slots keep the table at 8 MB | run-6ba0d0eeac90 | 0.088 | 21.8 | 63.8 |
| 4 | No quote-counting pass: by the spec's section 2.2 notes are lowercase words and quotes and names never hold an LF, so an LF followed by a digit or a BOM always ends a record, and each thread finds its first record start directly. `regions.csv` is mapped and parsed in parallel into a region-to-country-code array, with groups looked up through a 26^3 rank table, instead of 200,000 lines on one thread | run-e4f06a156ca2 | 0.081 | 14.9 | 66.0 |
| 5 | Hide the name table's cache miss: compute each name's key and home slot as soon as it is parsed and prefetch that slot's line, then update the previous record's name, whose line has had a whole record's parsing to arrive | run-9afc9ed38224 | 0.070 | 14.7 | 67.5 |
| 6 | Take the 8 MB name table as zeroed pages from the allocator instead of writing it on one thread before the parse; the parsing threads fault the pages in as they touch them | run-d3ed05cbee11 | 0.070 | 14.7 | 67.6 |
| 7 | Finish each record one record later: its region's entry in the 2 MB region-to-country array is prefetched along with its name slot, and both are used while the next record is parsed | run-a6b32c0a65ee | 0.068 | 14.6 | 67.7 |
| 8 | Structural scanning: SSE2 compares build 64-byte masks of quotes, commas and LFs, a prefix XOR of the quote mask marks quoted bytes, and the record loop reads its five delimiters from the remaining bits, with field lengths from quote counts. Group masks moved out of the shared name table: parsing runs in rounds, each record's (name slot, group) goes to a bucket for the thread that owns that slot's masks, and after a barrier each owner sets its own bits with plain writes, so no cache line is written by two cores | run-235961e6489a | 0.060 | 17.0 | 67.8 |
| 9 | AVX2 masks, delimiters scanned a 64 KB stretch ahead into a position array (one hard-to-predict branch per 64-byte block rather than per field), and numbers parsed without loops (one 8-byte load, digit pairs combined by multiplies). Reverted: no faster on the board, and S rose from 516 to 573 | run-739ff375d24e | 0.063 | 17.7 | 66.7 |
| 10 | Step 8's code again. Steps 8 and 10 are the same build, so 0.060 s to 0.062 s is the board's run-to-run spread | run-030a52ef2645 | 0.062 | 17.5 | 67.4 |
| 11 | Tighter record loop: numbers parsed from one masked 8-byte load ending at the field's end, digit pairs combined by multiplies; a name's key built from three masked word loads instead of a copy; the per-record finishing step inlined and the scanner state kept in locals, so the loop no longer copies a record tuple through the stack or reloads its state from memory | run-87949b19dc88 | 0.048 | 17.0 | 70.2 |
| 12 | `exit(0)` right after writing the output, so no destructors run. Not kept: 0.0517 s and 0.0515 s (run-881358ff5946) against 0.0509 s for step 11 measured in between (run-ca66c0a751e8) | run-9356ca046175 | 0.052 | 17.2 | 69.3 |
| 13 | Each thread maps its share of the file with `madvise(MADV_POPULATE_READ)` before parsing. Reverted | run-3e55edd37b4d | 0.058 | 16.3 | 68.2 |
| 14 | Half as many threads (6). Reverted | run-551f7fb4ea50 | 0.084 | 14.3 | 64.8 |
| 15 | Twice as many threads (24). Reverted | run-c9fcf1def886 | 0.056 | 19.5 | 67.8 |
| 16 | Read `records.csv` with `pread` into one reused 64 KB buffer per thread instead of mapping it, so there is no 200 MB mapping to tear down at exit: each piece is parsed up to its last whole record and the rest is read again with the next piece, and a name over 24 bytes keeps its tail in an append-only arena. Reverted: 0.0502 s against 0.0495 s for step 11 just before it (run-77a0854beb2e), and S rose from 528 to 578 | run-de787d452975 | 0.050 | 17.3 | 69.3 |

## Notes

- Step 1 checked against an independent `csv`-module oracle on the spec's worked example, 160 random hostile files (BOMs mid-file, CRLF/LF mix, quoted commas, `""` escapes, multi-line notes, notes starting or ending with a quote, empty quoted and unquoted fields, trailing-space name twins, negative amounts under 1.00, unknown regions) and a 3.32M-record synthetic file.
- Laptop: about 0.77 s on the 3.32M-record file, of which about 0.1 s is Windows reading the file (Linux maps it instead).
- Step 1 profile (laptop, one thread): parsing 1.3 s without name tracking and 2.8 s with it; every thread also kept its own copy of the ~100,000-name pool with group masks (about 7 MB each), which is most of the 103 MB.
- Step 2 on the laptop, 8 threads: regions 140 ms to 15 ms, quote count 90 ms to 15 ms, parse 450 ms to 130-215 ms.
- Step 2 on the board: T 207 ms to 90 ms and M 103 MB to 22 MB.
- Step 2 profile (laptop, one thread): parse 750 ms with name tracking, 330 ms without. Name tracking cost three cache misses per record: the table slot, the first occurrence's bytes back in the 200 MB file (to confirm a match), and the group mask in a separate 8 MB array.
- Step 3 on the laptop: name tracking 420 ms to 320 ms single-threaded; the 8-thread parse about 115 ms to 100 ms.
- Step 3 on the board: T 90 ms to 88 ms, a smaller gain than on the laptop; at 12 threads the serial steps around the parse (regions on one thread, the quote-counting pass) are a bigger share.
- Step 4 on the board: T 88 ms to 81 ms, and M 21.8 MB to 14.9 MB because `regions.csv` is mapped instead of read and copied into a row list.
- Step 5 on the board: T 81 ms to 70 ms, with the same memory.
- Step 6 on the board: no measurable change (70.3 ms to 69.8 ms), so zero-filling the table on one thread was not a real cost.
- Step 7 board profile (run-45920caff785, an instrumented build that writes its phase times into an extra file name, so the run fails the output check on purpose): regions 1.7 ms, parse 49.6 ms, distinct count 7.3 ms. The same build with name tracking switched off (run-3ef3af02445e) parsed in 34.2 ms and finished in 36.7 ms instead of 58.9 ms, so the shared name table cost about 22 ms. Each atomic OR on a group mask invalidated the line that other threads read the name's key from. Step 8 separates the two.
- After step 8, a laptop breakdown with one thread: SIMD scanning alone 78 ms over 223 MB (2.9 GB/s), field extraction and number parsing +100 ms, name hashing +15 ms, group aggregates +20 ms, name slots and mask buckets +125 ms.
- Board profile of step 10 (run-3dc00e72cdef, instrumented, fails the output check on purpose): regions 1.8 ms; in the parse phase each thread spent 45.4 ms parsing (maximum 46.3), 2.7 ms setting its own mask bits and 1.7 ms at barriers; about 6 ms outside `main`. About 160 ns per record per thread, so the record loop itself was the target.
- Before step 11, the assembly showed the finishing closure compiled as a separate call taking an 88-byte tuple, and the scanner's bit state re-read from the stack on each delimiter. On the laptop, interleaved A/B of the parse phase: 8 threads 85-97 ms to 72-81 ms; 1 thread 309 ms to 260 ms (minimum). Loop-free numbers alone were worth 3-7%, which is why step 9, which had them without the rest, did not show on the board.
- Tried before step 11 and not kept: a carry-less-multiply prefix XOR (no gain); a private name table per owner fed by the buckets (slower, because owners miss on the name bytes in the file); 2^18 name slots (no gain, twice the memory); prefetching the next slot line as well (no gain).
- Ablations at 8 threads on the laptop: removing all name work took the parse from 85-90 ms to 54-59 ms (probing about 12-15 ms, slot access, bucket push and owner masks about 10 ms, hashing about 10 ms).
- After step 11, 8 threads on the laptop (parse phase, interleaved A/B): baseline 72.6-74 ms; without the owners' mask writes 63-69 ms; without the bucket push 69-70.5 ms; without the name probe loop about 57-63 ms. The board profile puts the owner phase at only 2.7 ms, so what remains of the name cost is mostly the miss on the slot line.
- Tried after step 11 and not kept, none measurably faster: an owner-side name table fed with the 24-byte keys (much slower, 209-241 ms against 141-163 ms in the same batch); a one-byte BOM check, unchecked number loads and the region's group stored directly in `code_of` (within noise); dropping the one-record deferral (slower, 64.8/83.9 ms against 62.3/72.8 ms min/median, so it stays). A tag-byte array was not built: the whole probe loop is worth at most about 6 ms of 72 ms on the laptop.
- Board profile of step 11 (run-f68866234c17, instrumented): regions 1.9 ms, split 2.0 ms, parse phase done at 46.7 ms with per-thread parse time 32.9-37.3 ms, barrier waits 2.3-6.5 ms and owner work 2.1-2.6 ms. Explicitly unmapping the 200 MB input took 4.8 ms, which is most of the time previously seen outside `main` (normally paid at exit). Steps 12-15 tested the obvious levers and none helped, so the current code is step 11.
- Late on Oct 31 the board's timings became much noisier: step 11's code measured 0.048 s, 0.051 s and 0.071 s (run-b5081feba568) on three runs.
- Step 16 targeted the 4.8 ms unmap from the step-11 profile. Avoiding the mapping saved that, but each thread then copies its 17 MB itself, and the board showed no net gain.
