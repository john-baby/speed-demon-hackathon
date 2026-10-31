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

// The commas and LFs outside quotes, in file order, found 64 bytes at a time. A byte is inside
// quotes when an odd number of quotes precede it, so a running XOR over the block's quote mask (with
// the carry from the block before) marks the quoted bytes; "" escapes cancel themselves out.
#[derive(Clone, Copy)]
struct Fields<'a> {
    d: &'a [u8],
    block: usize,  // start of the current 64-byte block
    bits: u64,     // its commas and LFs outside quotes not yet returned
    quotes: u64,   // its quote bytes
    qbase: usize,  // quote bytes before it
    inside: u64,   // all ones when it ends inside quotes
}

impl<'a> Fields<'a> {
    fn new(d: &'a [u8], start: usize) -> Self {
        let mut f = Fields { d, block: start, bits: 0, quotes: 0, qbase: 0, inside: 0 };
        f.load();
        f
    }

    fn load(&mut self) {
        let mut pad = [0u8; 64];
        let src = if self.block + 64 <= self.d.len() {
            &self.d[self.block..self.block + 64]
        } else {
            let tail = &self.d[self.block.min(self.d.len())..];
            pad[..tail.len()].copy_from_slice(tail);
            &pad[..]
        };
        let [q, c, n] = [b'"', b',', b'\n'].map(|b| mask(src, b));
        let mut x = q;
        for k in [1, 2, 4, 8, 16, 32] {
            x ^= x << k;
        }
        let inq = x ^ self.inside;
        self.inside = ((inq as i64) >> 63) as u64;
        self.quotes = q;
        self.bits = (c | n) & !inq;
    }

    // The next comma or LF outside quotes, and how many quote bytes precede it.
    fn next(&mut self) -> (usize, usize) {
        while self.bits == 0 {
            self.qbase += self.quotes.count_ones() as usize;
            self.block += 64;
            self.load();
        }
        let off = self.bits.trailing_zeros();
        self.bits &= self.bits - 1;
        (self.block + off as usize, self.qbase + (self.quotes & ((1u64 << off) - 1)).count_ones() as usize)
    }
}

// Bit i set where the 64 bytes at s hold b, 16 bytes per SSE2 compare.
fn mask(s: &[u8], b: u8) -> u64 {
    use std::arch::x86_64::*;
    let mut m = 0u64;
    for k in 0..4 {
        let bits = unsafe { _mm_movemask_epi8(_mm_cmpeq_epi8(_mm_loadu_si128(s[16 * k..].as_ptr() as *const __m128i), _mm_set1_epi8(b as i8))) };
        m |= (bits as u16 as u64) << (16 * k);
    }
    m
}

// The 1 to 8 digits in d[s..e], without a loop: one 8-byte load ending at e, the bytes before s
// masked off, then digit pairs, quads and octets combined by multiplies.
fn digits(d: &[u8], s: usize, e: usize) -> i64 {
    let x = (u64::from_le_bytes(d[e - 8..e].try_into().unwrap()) ^ 0x3030_3030_3030_3030) & u64::MAX << (8 * (8 - (e - s)));
    let x = (x.wrapping_mul(10) + (x >> 8)) & 0x00ff_00ff_00ff_00ff;
    let x = (x.wrapping_mul(100) + (x >> 16)) & 0x0000_ffff_0000_ffff;
    ((x.wrapping_mul(10000) + (x >> 32)) & 0xffff_ffff) as i64
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

// Distinct names across all threads: a lock-free open-addressing table of half-cache-line slots,
// each holding the name's file offset and length (claimed by compare-and-swap) and its first 24 bytes.
// Slots are written once, so after the first few thousand records every thread only reads them.
#[repr(C, align(32))]
struct Slot {
    entry: AtomicU64, // offset << 32 | len + 1; 0 = empty, BUSY = being filled
    key: [AtomicU64; 3],
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
        // Three word loads masked to the name's length; only a name in the file's last 24 bytes is copied.
        let mut b = [0u8; 24];
        let src = if s + 24 <= d.len() { &d[s..s + 24] } else { b[..d.len() - s].copy_from_slice(&d[s..]); &b };
        let key: [u64; 3] = std::array::from_fn(|k| {
            let n = len.saturating_sub(8 * k);
            u64::from_le_bytes(src[8 * k..8 * k + 8].try_into().unwrap()) & if n >= 8 { !0 } else { !(!0 << (8 * n)) }
        });
        let h = (key[0] ^ key[1].rotate_left(21) ^ key[2].rotate_left(42) ^ len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let i = (h >> (64 - NAME_BITS)) as usize;
        prefetch(&self.slots[i]);
        Lookup { s, len, key, i }
    }

    // The name's slot, inserting it if new.
    #[inline(always)]
    fn slot(&self, d: &[u8], l: &Lookup) -> usize {
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
                return i;
            }
            i = (i + 1) & ((1 << NAME_BITS) - 1);
        }
    }
}

// A thread's share of records.csv, parsed a round at a time. Each record's (name slot, group) pair
// goes to the bucket of the thread that owns that slot's group mask, so masks are only ever written
// by their owner and no cache line bounces between cores.
struct Chunk<'a> {
    f: Fields<'a>,
    p: usize,
    end: usize,
    aggs: Vec<Agg>,
}

struct Ctx<'a> {
    d: &'a [u8],
    code_of: &'a [u16],
    rank: &'a [u16],
    unmatched: usize,
    names: &'a Names,
}

impl<'a> Chunk<'a> {
    // Parse the records starting before `until` (record starts are outside quotes) and aggregate them
    // by group: rank[code_of[region_id]] is the region's group, and a missing region is `unmatched`.
    fn run(&mut self, c: &Ctx, until: usize, out: &mut [Vec<u32>]) {
        let d = c.d;
        let mut pending = None;
        #[inline(always)]
        fn finish(c: &Ctx, aggs: &mut [Agg], out: &mut [Vec<u32>], (l, region, id, cents, name_bytes, bytes): (Lookup, usize, i64, i64, usize, i64)) {
            let g = c.code_of.get(region).filter(|&&k| k != NONE).map_or(c.unmatched, |&k| c.rank[k as usize] as usize);
            let a = &mut aggs[g];
            a.count += 1;
            a.id_sum += id;
            a.amount_sum += cents;
            a.min = a.min.min(cents);
            a.max = a.max.max(cents);
            a.name_bytes += name_bytes as i64;
            a.note_bytes += bytes - name_bytes as i64;
            let i = c.names.slot(c.d, &l);
            out[i * out.len() >> NAME_BITS].push((i as u32) << 8 | g as u32);
        }
        // A field of n bytes holding q quote bytes is a value of n bytes when q is 0, and of
        // n - 1 - q/2 bytes when quoted: two delimiting quotes and one byte saved per "" escape.
        let value_len = |n: usize, q: usize| if q == 0 { n } else { n - 1 - q / 2 };
        let (mut f, mut p) = (self.f, self.p);
        let until = until.min(self.end);
        while p < until {
            p += if d[p..].starts_with(BOM) { 3 } else { 0 };
            let ((c1, q1), (c2, q2), (c3, q3), (c4, _), (lf, _)) = (f.next(), f.next(), f.next(), f.next(), f.next());
            let id = digits(d, p, c1);
            let name = if q2 > q1 { &d[c1 + 2..c2 - 1] } else { &d[c1 + 1..c2] };
            let name_len = value_len(c2 - c1 - 1, q2 - q1);
            let note_len = value_len(c3 - c2 - 1, q3 - q2);
            let region = digits(d, c3 + 1, c4) as usize;
            let end = lf - (d[lf - 1] == b'\r') as usize;
            let negative = d[c4 + 1] == b'-';
            let cents = (digits(d, c4 + 1 + negative as usize, end - 3) * 100 + digits(d, end - 2, end)) * if negative { -1 } else { 1 };
            p = lf + 1;

            // Finish the previous record while this one's region and name lines are on their way in.
            if let Some(r) = c.code_of.get(region) {
                prefetch(r);
            }
            let l = c.names.lookup(d, name.as_ptr() as usize - d.as_ptr() as usize, name.len());
            if let Some(prev) = pending.replace((l, region, id, cents, name_len, (name_len + note_len) as i64)) {
                finish(c, &mut self.aggs, out, prev);
            }
        }
        if let Some(prev) = pending {
            finish(c, &mut self.aggs, out, prev);
        }
        (self.f, self.p) = (f, p);
    }
}

// A value threads hand each other between barriers: each round, thread t fills its own buckets,
// then (after a barrier) every owner reads the buckets addressed to it.
struct Cell<T>(std::cell::UnsafeCell<T>);
unsafe impl<T> Sync for Cell<T> {}

fn money(c: i64) -> String {
    format!("{}{}.{:02}", if c < 0 { "-" } else { "" }, c.abs() / 100, c.abs() % 100)
}

static PROF: std::sync::Mutex<Vec<(usize, u128, u128, u128, i32, i32)>> = std::sync::Mutex::new(Vec::new());
#[cfg(unix)]
extern "C" {
    fn sched_getcpu() -> i32;
    fn munmap(addr: *const u8, len: usize) -> i32;
}
#[cfg(not(unix))]
unsafe fn sched_getcpu() -> i32 { -1 }
fn main() {
    let t0 = std::time::Instant::now();
    let mut marks = String::new();
    let us = |t: std::time::Instant| t.elapsed().as_micros();
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

    marks += &format!("_rg{}", us(t0));
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

    marks += &format!("_sp{}", us(t0));
    // 3. Parse and aggregate every chunk in parallel, all threads sharing one name table, in rounds
    //    of about ROUND bytes per thread so the buckets stay in cache.
    let groups = unmatched + 1;
    assert!(groups <= 256, "group masks hold 256 groups");
    // All-zero is an empty slot, so take zeroed pages from the allocator rather than writing 4 MB
    // on one thread; the parsing threads fault them in as they touch them.
    let names = Names {
        slots: unsafe {
            let n = 1 << NAME_BITS;
            Vec::from_raw_parts(std::alloc::alloc_zeroed(std::alloc::Layout::array::<Slot>(n).unwrap()) as *mut Slot, n, n)
        },
    };
    const ROUND: usize = 1 << 20;
    let rounds = (0..nt).map(|i| starts[i + 1] - starts[i]).max().unwrap_or(0).div_ceil(ROUND).max(1);
    let buckets: Vec<Cell<Vec<Vec<u32>>>> = (0..nt).map(|_| Cell(std::cell::UnsafeCell::new(vec![Vec::new(); nt]))).collect();
    let barrier = std::sync::Barrier::new(nt);
    let ctx = Ctx { d, code_of: &code_of, rank: &rank, unmatched, names: &names };
    let parts: Vec<(Vec<Agg>, Vec<i64>)> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|t| {
            let (s, e, ctx, buckets, barrier) = (starts[t], starts[t + 1], &ctx, &buckets, &barrier);
            sc.spawn(move || {
                let empty = Agg { count: 0, id_sum: 0, amount_sum: 0, min: i64::MAX, max: i64::MIN, name_bytes: 0, note_bytes: 0 };
                let mut chunk = Chunk { f: Fields::new(d, s), p: s, end: e, aggs: vec![empty; groups] };
                // This thread's group masks: those of slots lo..hi.
                let (lo, hi) = (((t << NAME_BITS) + nt - 1) / nt, (((t + 1) << NAME_BITS) + nt - 1) / nt);
                let mut masks = vec![[0u64; 4]; hi - lo];
                let cpu0 = unsafe { sched_getcpu() };
                let (mut rn, mut ow, mut bw) = (0, 0, 0);
                for r in 0..rounds {
                    let out = unsafe { &mut *buckets[t].0.get() };
                    out.iter_mut().for_each(Vec::clear);
                    let ta = std::time::Instant::now();
                    chunk.run(ctx, s + (e - s) * (r + 1) / rounds, out);
                    rn += us(ta);
                    let ta = std::time::Instant::now();
                    barrier.wait();
                    bw += us(ta);
                    let ta = std::time::Instant::now();
                    for b in buckets {
                        for &x in &unsafe { &*b.0.get() }[t] {
                            let (i, g) = ((x >> 8) as usize - lo, (x & 255) as usize);
                            masks[i][g / 64] |= 1 << (g % 64);
                        }
                    }
                    ow += us(ta);
                    let ta = std::time::Instant::now();
                    barrier.wait();
                    bw += us(ta);
                }
                PROF.lock().unwrap().push((t, rn, ow, bw, cpu0, unsafe { sched_getcpu() }));
                // 4. Each group's distinct names are the masks with its bit set.
                let mut c = vec![0i64; groups];
                for m in &masks {
                    for (w, &word) in m.iter().enumerate() {
                        let mut bits = word;
                        while bits != 0 {
                            c[w * 64 + bits.trailing_zeros() as usize] += 1;
                            bits &= bits - 1;
                        }
                    }
                }
                (chunk.aggs, c)
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    marks += &format!("_pa{}", us(t0));
    {
        let v = PROF.lock().unwrap();
        let f = |k: usize| { let x: Vec<u128> = v.iter().map(|e| [e.1, e.2, e.3][k]).collect(); (x.iter().min().copied().unwrap_or(0), x.iter().max().copied().unwrap_or(0)) };
        let ((a, b), (c, d2), (e, g)) = (f(0), f(1), f(2));
        marks += &format!("_nt{nt}_run{a}-{b}_own{c}-{d2}_bar{e}-{g}_cpu");
        let mut cpus: Vec<i32> = v.iter().flat_map(|e| [e.4, e.5]).collect();
        cpus.sort();
        cpus.dedup();
        marks += &cpus.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("-");
    }
    let counts: Vec<&Vec<i64>> = parts.iter().map(|p| &p.1).collect();
    let parts: Vec<&Vec<Agg>> = parts.iter().map(|p| &p.0).collect();
    let distinct: Vec<i64> = (0..groups).map(|g| counts.iter().map(|c| c[g]).sum()).collect();

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
    marks += &format!("_wr{}", us(t0));
    let ta = std::time::Instant::now();
    #[cfg(unix)]
    unsafe { munmap(d.as_ptr(), d.len()); }
    marks += &format!("_um{}", us(ta));
    std::fs::write(format!("{out_dir}/prof{marks}_end{}", us(t0)), b"").unwrap();
}
