// Speed Demon 2026, Track C (Join). See README.md for the approach.
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering::{Acquire, Relaxed, Release}};
use std::thread;

// Map the input read-only; the host has it in the page cache, so it is not charged to us.
#[cfg(unix)]
fn load(path: &str) -> &'static [u8] {
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    }
    let f = std::fs::File::open(path).unwrap();
    let len = f.metadata().unwrap().len() as usize;
    if len == 0 {
        return &[];
    }
    let p = unsafe { mmap(std::ptr::null_mut(), len, 1, 2, f.as_raw_fd(), 0) }; // PROT_READ, MAP_PRIVATE
    assert!(p as isize != -1, "mmap failed");
    unsafe { std::slice::from_raw_parts(p, len) }
}

#[cfg(not(unix))]
fn load(path: &str) -> &'static [u8] {
    std::fs::read(path).unwrap().leak()
}

const BOM: &[u8] = b"\xef\xbb\xbf";
const NONE: u16 = u16::MAX;

// The first line start at or after byte i of an LF-terminated file.
fn line_start(d: &[u8], i: usize) -> usize {
    if i == 0 || d[i - 1] == b'\n' { i } else { find(d, i, b'\n') + 1 }
}

#[derive(Clone, Copy)]
struct Shared(*mut u16);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}
impl Shared {
    fn at(self, i: usize) -> *mut u16 {
        unsafe { self.0.add(i) }
    }
}

// Index of the first `b` at or after `i`, 8 bytes at a time.
fn find(d: &[u8], mut i: usize, b: u8) -> usize {
    while i + 8 <= d.len() {
        let w = u64::from_le_bytes(d[i..i + 8].try_into().unwrap()) ^ (b as u64 * 0x0101_0101_0101_0101);
        let z = w.wrapping_sub(0x0101_0101_0101_0101) & !w & 0x8080_8080_8080_8080;
        if z != 0 {
            return i + z.trailing_zeros() as usize / 8;
        }
        i += 8;
    }
    i + d[i..].iter().position(|&c| c == b).unwrap()
}

// The unsigned number at *p, stepping past the one separator after it.
fn num(d: &[u8], p: &mut usize) -> i64 {
    let mut x = 0;
    while d[*p].is_ascii_digit() {
        x = x * 10 + (d[*p] - b'0') as i64;
        *p += 1;
    }
    *p += 1;
    x
}

// A text field at *p: its raw bytes (between the quotes, if quoted) and how many "" escapes they
// hold, with *p moved past the comma after it. Quoting is canonical for non-empty values, so the
// raw bytes identify the value, and its length is the raw length minus the escapes.
fn text<'a>(d: &'a [u8], p: &mut usize) -> (&'a [u8], usize) {
    let s = *p;
    if d[s] != b'"' {
        let e = find(d, s, b',');
        *p = e + 1;
        return (&d[s..e], 0);
    }
    let (mut i, mut escapes) = (s + 1, 0);
    loop {
        i = find(d, i, b'"');
        if d[i + 1] != b'"' {
            break;
        }
        escapes += 1;
        i += 2;
    }
    *p = i + 2;
    (&d[s + 1..i], escapes)
}

#[derive(Clone, Copy)]
struct Agg {
    count: i64,
    id_sum: i64,
    amount_sum: i64,
    min: i64,
    max: i64,
    name_bytes: i64,
    note_bytes: i64,
}

// Distinct names across all threads: a lock-free open-addressing table of one-cache-line slots.
// A slot holds the name's file offset and length (claimed by compare-and-swap), its first 24 bytes,
// and its bitmask of the groups it occurs in, so a lookup touches one line, not three.
#[repr(C, align(64))]
struct Slot {
    entry: AtomicU64, // offset << 32 | len + 1; 0 = empty, BUSY = being filled
    key: [AtomicU64; 3],
    mask: [AtomicU64; 4],
}

const NAME_BITS: u32 = 17; // the generator draws names from a pool of 100,000
const BUSY: u64 = 1;

struct Names {
    slots: Vec<Slot>,
}

// Ask for the cache line holding r, without waiting for it.
fn prefetch<T>(r: &T) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::*;
        _mm_prefetch(r as *const T as *const i8, _MM_HINT_T0);
    }
}

// A name's key and home slot, computed one record ahead so its cache line can be prefetched.
struct Lookup {
    s: usize,
    len: usize,
    key: [u64; 3],
    i: usize,
}

impl Names {
    fn lookup(&self, d: &[u8], s: usize, len: usize) -> Lookup {
        let mut b = [0u8; 24];
        b[..len.min(24)].copy_from_slice(&d[s..s + len.min(24)]);
        let key: [u64; 3] = std::array::from_fn(|i| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap()));
        let h = (key[0] ^ key[1].rotate_left(21) ^ key[2].rotate_left(42) ^ len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let i = (h >> (64 - NAME_BITS)) as usize;
        prefetch(&self.slots[i]);
        Lookup { s, len, key, i }
    }

    fn add(&self, d: &[u8], l: &Lookup, g: usize) {
        let Lookup { s, len, key, mut i } = *l;
        let entry = (s as u64) << 32 | (len as u64 + 1);
        loop {
            let slot = &self.slots[i];
            let mut e = slot.entry.load(Acquire);
            if e == 0 {
                match slot.entry.compare_exchange(0, BUSY, Acquire, Acquire) {
                    Ok(_) => {
                        for (k, &w) in slot.key.iter().zip(&key) {
                            k.store(w, Relaxed);
                        }
                        slot.entry.store(entry, Release);
                        e = entry;
                    }
                    Err(now) => e = now,
                }
            }
            while e == BUSY {
                e = slot.entry.load(Acquire);
            }
            let o = (e >> 32) as usize;
            if e & 0xffff_ffff == len as u64 + 1
                && slot.key.iter().zip(&key).all(|(k, &w)| k.load(Relaxed) == w)
                && (len <= 24 || d[o + 24..o + len] == d[s + 24..s + len])
            {
                let m = &slot.mask[g / 64];
                if m.load(Relaxed) >> (g % 64) & 1 == 0 {
                    m.fetch_or(1 << (g % 64), Relaxed);
                }
                return;
            }
            i = (i + 1) & ((1 << NAME_BITS) - 1);
        }
    }
}

// Parse the records in d[start..end] (start is a record start, outside quotes) and aggregate them
// by group: rank[code_of[region_id]] is the region's group, and a missing region is group `unmatched`.
fn parse(d: &[u8], start: usize, end: usize, code_of: &[u16], rank: &[u16], unmatched: usize, names: &Names) -> Vec<Agg> {
    let empty = Agg { count: 0, id_sum: 0, amount_sum: 0, min: i64::MAX, max: i64::MIN, name_bytes: 0, note_bytes: 0 };
    let mut aggs = vec![empty; unmatched + 1];
    let mut pending = None;
    let finish = |aggs: &mut [Agg], (l, region, id, cents, name_bytes, bytes): (Lookup, usize, i64, i64, usize, i64)| {
        let g = code_of.get(region).filter(|&&c| c != NONE).map_or(unmatched, |&c| rank[c as usize] as usize);
        let a = &mut aggs[g];
        a.count += 1;
        a.id_sum += id;
        a.amount_sum += cents;
        a.min = a.min.min(cents);
        a.max = a.max.max(cents);
        a.name_bytes += name_bytes as i64;
        a.note_bytes += bytes - name_bytes as i64;
        names.add(d, &l, g);
    };
    let mut p = start;
    while p < end {
        if d[p..].starts_with(BOM) {
            p += 3;
        }
        let id = num(d, &mut p);
        let (name, name_esc) = text(d, &mut p);
        let (note, note_esc) = text(d, &mut p);
        let region = num(d, &mut p) as usize;
        let negative = d[p] == b'-';
        p += negative as usize;
        let whole = num(d, &mut p);
        let cents = (whole * 100 + (d[p] - b'0') as i64 * 10 + (d[p + 1] - b'0') as i64) * if negative { -1 } else { 1 };
        p += 2;
        p += if d.get(p) == Some(&b'\r') { 2 } else { 1 };

        // Finish the previous record while this one's region and name lines are on their way in.
        if let Some(r) = code_of.get(region) {
            prefetch(r);
        }
        let l = names.lookup(d, name.as_ptr() as usize - d.as_ptr() as usize, name.len());
        let bytes = (name.len() - name_esc + note.len() - note_esc) as i64;
        if let Some(prev) = pending.replace((l, region, id, cents, name.len() - name_esc, bytes)) {
            finish(&mut aggs, prev);
        }
    }
    if let Some(prev) = pending {
        finish(&mut aggs, prev);
    }
    aggs
}

fn money(c: i64) -> String {
    format!("{}{}.{:02}", if c < 0 { "-" } else { "" }, c.abs() / 100, c.abs() % 100)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = args.get(1).map_or("/data", |s| s.as_str());
    let out_dir = args.get(2).map_or("/out", |s| s.as_str());
    let nt = thread::available_parallelism().map_or(1, |n| n.get());

    // 1. Regions, in parallel: code_of[region_id] is the region's country code as a number below
    //    26^3, and rank[code] its group (the code's place in sorted order); the threads fill
    //    disjoint ids. A region id not listed maps to `unmatched`.
    let r = load(&format!("{data_dir}/regions.csv"));
    let body = find(r, 0, b'\n') + 1;
    let mut p = r[..r.len() - 1].iter().rposition(|&b| b == b'\n').map_or(0, |x| x + 1).max(body);
    let mut code_of = vec![NONE; if p < r.len() { num(r, &mut p) as usize + 1 } else { 0 }];
    let out = Shared(code_of.as_mut_ptr());
    let present: Vec<[bool; 17576]> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (mut p, end) = (line_start(r, body + (r.len() - body) * i / nt), line_start(r, body + (r.len() - body) * (i + 1) / nt));
            sc.spawn(move || {
                let mut seen = [false; 17576];
                while p < end {
                    let id = num(r, &mut p);
                    let c = ((r[p] - b'A') as usize * 26 + (r[p + 1] - b'A') as usize) * 26 + (r[p + 2] - b'A') as usize;
                    seen[c] = true;
                    unsafe { *out.at(id as usize) = c as u16 };
                    p += 4;
                }
                seen
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut rank = vec![0u16; 17576];
    let mut codes = Vec::new();
    for (k, r) in rank.iter_mut().enumerate() {
        if present.iter().any(|s| s[k]) {
            *r = codes.len() as u16;
            codes.push([b'A' + (k / 676) as u8, b'A' + (k / 26 % 26) as u8, b'A' + (k % 26) as u8]);
        }
    }
    let unmatched = codes.len();

    // 2. Split records.csv at record starts. Notes are lowercase words and quotes and names never
    //    hold an LF, so an LF followed by a digit or a BOM always ends a record.
    let d = load(&format!("{data_dir}/records.csv"));
    let header = d.strip_prefix(BOM).map_or(0, |_| 3);
    let mut starts = vec![find(d, header, b'\n') + 1];
    for i in 1..nt {
        let mut x = (d.len() * i / nt).max(starts[i - 1]);
        while x < d.len() {
            x = find(d, x, b'\n') + 1;
            if x == d.len() || d[x].is_ascii_digit() || d[x] == BOM[0] {
                break;
            }
        }
        starts.push(x.min(d.len()));
    }
    starts.push(d.len());

    // 3. Parse and aggregate every chunk in parallel, all threads sharing one name table.
    let groups = unmatched + 1;
    assert!(groups <= 256, "group masks hold 256 groups");
    // All-zero is an empty slot, so take zeroed pages from the allocator rather than writing 8 MB
    // on one thread; the parsing threads fault them in as they touch them.
    let names = Names {
        slots: unsafe {
            let n = 1 << NAME_BITS;
            Vec::from_raw_parts(std::alloc::alloc_zeroed(std::alloc::Layout::array::<Slot>(n).unwrap()) as *mut Slot, n, n)
        },
    };
    let parts: Vec<Vec<Agg>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (s, e, code_of, rank, names) = (starts[i], starts[i + 1], &code_of, &rank, &names);
            sc.spawn(move || parse(d, s, e, code_of, rank, unmatched, names))
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 4. Each group's distinct names are the masks with its bit set.
    let mut distinct = vec![0i64; groups];
    for slot in &names.slots {
        for (w, word) in slot.mask.iter().enumerate() {
            let mut bits = word.load(Relaxed);
            while bits != 0 {
                distinct[w * 64 + bits.trailing_zeros() as usize] += 1;
                bits &= bits - 1;
            }
        }
    }

    let mut o = Vec::new();
    for g in 0..groups {
        let mut a = parts[0][g];
        for aggs in &parts[1..] {
            let b = aggs[g];
            a.count += b.count;
            a.id_sum += b.id_sum;
            a.amount_sum += b.amount_sum;
            a.min = a.min.min(b.min);
            a.max = a.max.max(b.max);
            a.name_bytes += b.name_bytes;
            a.note_bytes += b.note_bytes;
        }
        if a.count == 0 {
            continue;
        }
        let key = if g == unmatched { "_UNMATCHED" } else { std::str::from_utf8(&codes[g]).unwrap() };
        writeln!(o, "{key} {} {} {} {} {} {} {} {}", a.count, a.id_sum, money(a.amount_sum), money(a.min), money(a.max), distinct[g], a.name_bytes, a.note_bytes).unwrap();
    }
    std::fs::write(format!("{out_dir}/aggregates.txt"), o).unwrap();
}
