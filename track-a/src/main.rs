// Speed Demon 2026, Track A (Ingest). See README.md for the approach.
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::arch::x86_64::*;

const HOURS: usize = 72; // 2026-10-30T18:00Z .. 2026-11-02T17:59Z, fixed by the spec
const B: usize = 8; // latencies per block: few to waste in a group's last block

// Map the input read-only; the host has it in the page cache, so it is not charged to us.
#[cfg(unix)]
fn load(path: &str) -> &'static [u8] {
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
        fn prctl(op: i32, ...) -> i32;
    }
    // PR_SET_THP_DISABLE: transparent huge pages would round each thread's small, scattered
    // allocations up to 2 MB of resident memory.
    unsafe { prctl(41, 1u64, 0u64, 0u64, 0u64) };
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

fn word(d: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(d[i..i + 8].try_into().unwrap())
}

// Bit k is set where f marks byte k of the 64 (SSE2, so any x86-64 has it); f gets each
// 16-byte quarter and its number.
fn bits(b: &[u8; 64], f: impl Fn(__m128i, usize) -> __m128i) -> u64 {
    (0..4).fold(0, |m, k| {
        let v = unsafe { _mm_loadu_si128(b.as_ptr().add(16 * k) as *const __m128i) };
        m | (unsafe { _mm_movemask_epi8(f(v, k)) } as u16 as u64) << (16 * k)
    })
}

// The latency in d[a..q], q being its newline. Up to 8 digits are read as one word whose
// leading bytes are cleared (they then read as zeros) and combined by three multiplies.
fn latency(d: &[u8], a: usize, q: usize) -> u32 {
    if q - a > 8 {
        return d[a..q].iter().fold(0, |lat, &b| lat * 10 + (b - b'0') as u32);
    }
    let mut v = word(d, q - 8) & (u64::MAX << (8 * (8 - (q - a))));
    v = (v & 0x0f0f_0f0f_0f0f_0f0f).wrapping_mul(2561) >> 8;
    v = (v & 0x00ff_00ff_00ff_00ff).wrapping_mul(6_553_601) >> 16;
    ((v & 0x0000_ffff_0000_ffff).wrapping_mul(42_949_672_960_001) >> 32) as u32
}

fn hour(d: &[u8], p: usize) -> usize {
    let digit = |i: usize| (d[i] - b'0') as usize;
    let day = digit(p + 8) * 10 + digit(p + 9);
    (if day >= 30 { day - 30 } else { day + 1 }) * 24 + digit(p + 11) * 10 + digit(p + 12) - 18
}

// A group's line count, 5xx count and newest block of latencies.
#[derive(Clone, Copy, Default)]
struct Group {
    n: u32,
    errors: u32,
    block: u32,
}

// Endpoint names and their ids, in order of first appearance.
struct Names {
    names: Vec<Vec<u8>>, // copied, so comparing them later stays in cache
    pre: Vec<[u8; 64]>,  // the first 64 bytes of each, zero-padded
    slots: Vec<Slot>,    // open addressing; id 0 = empty
}

// One thread's view of its chunk, with its own endpoint ids.
struct Chunk {
    t: Names,
    groups: Vec<Vec<Group>>, // [hour][endpoint id], grown on demand: a fixed-size table was
                             // touched in full by probing every hour, about 3.5 MB per thread
    lats: Vec<[u32; B]>, // each group's latencies, in a chain of blocks
    prev: Vec<u32>,      // the block before each block in its group's chain
}

// Small, so the table stays in L1/L2; the name itself is checked against `pre`.
#[derive(Clone, Copy, Default)]
struct Slot {
    len: u32,
    id: u32, // id + 1
}

impl Names {
    fn new() -> Names {
        Names { names: Vec::new(), pre: Vec::new(), slots: vec![Slot::default(); 1 << 13] }
    }

    // The id of an endpoint already in memory, padded as `parse` reads it.
    fn id_of(&mut self, ep: &[u8]) -> usize {
        let mut head = [0; 8];
        head[..ep.len().min(8)].copy_from_slice(&ep[..ep.len().min(8)]);
        let tail = if ep.len() >= 8 { word(ep, ep.len() - 8) } else { 0 };
        self.id(ep, None, u64::from_le_bytes(head), tail)
    }

    // win is the 64 bytes from the start of ep, when the input has that many.
    fn id(&mut self, ep: &[u8], win: Option<&[u8; 64]>, head: u64, tail: u64) -> usize {
        let len = ep.len();
        let h = (head ^ tail.rotate_left(29) ^ len as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        let mask = self.slots.len() - 1;
        let mut i = (h >> 40) as usize & mask;
        loop {
            let s = self.slots[i];
            if s.id == 0 {
                self.names.push(ep.to_vec());
                let mut pre = [0; 64];
                pre[..len.min(64)].copy_from_slice(&ep[..len.min(64)]);
                self.pre.push(pre);
                self.slots[i] = Slot { len: len as u32, id: self.names.len() as u32 };
                assert!(self.names.len() < self.slots.len() / 2);
                return self.names.len() - 1;
            }
            let id = s.id as usize - 1;
            if s.len as usize == len && match win {
                // One fixed-size compare: a loop or a memcmp call here mispredicts on the length.
                Some(w) if len <= 64 => {
                    let p = self.pre[id].as_ptr() as *const __m128i;
                    !bits(w, |v, k| unsafe { _mm_cmpeq_epi8(v, _mm_loadu_si128(p.add(k))) }) & u64::MAX >> (64 - len) == 0
                }
                _ => ep == self.names[id],
            } {
                return id;
            }
            i = (i + 1) & mask;
        }
    }
}

impl Chunk {
    // Appends the latencies of group g to v: the newest block holds the last (n - 1) % B + 1,
    // every older one is full. Whole blocks are copied, as fixed-size moves.
    fn gather(&self, g: Group, v: &mut Vec<u32>) {
        let (n, mut b) = (g.n as usize, g.block as usize);
        v.extend_from_slice(&self.lats[b]);
        v.truncate(v.len() - (B - 1 - (n - 1) % B));
        for _ in 0..(n - 1) / B {
            b = self.prev[b] as usize;
            v.extend_from_slice(&self.lats[b]);
        }
    }
}

// The only pass over the input: intern each line's endpoint and append its latency to its group.
fn parse(d: &[u8]) -> Chunk {
    let blocks = d.len() / 512; // about what typical lines need
    let mut c = Chunk { t: Names::new(), groups: vec![Vec::new(); HOURS],
        lats: Vec::with_capacity(blocks), prev: Vec::with_capacity(blocks) };
    // Every line has exactly four spaces and a newline, so the separators, in order, come in
    // fives and no field is ever searched for. They are collected 64 bytes at a time.
    let (mut sep, mut n) = ([0usize; 192], 0);
    for at in (0..d.len()).step_by(64) {
        let mut b = [0; 64];
        let w = if at + 64 <= d.len() { d[at..at + 64].try_into().unwrap() } else {
            b[..d.len() - at].copy_from_slice(&d[at..]);
            &b
        };
        let (sp, nl) = unsafe { (_mm_set1_epi8(b' ' as i8), _mm_set1_epi8(b'\n' as i8)) };
        let mut m = bits(w, |v, _| unsafe { _mm_or_si128(_mm_cmpeq_epi8(v, sp), _mm_cmpeq_epi8(v, nl)) });
        // Write eight positions whatever the count (a block rarely has more), so the loop
        // does not mispredict on how many there are.
        let end = n + m.count_ones() as usize;
        while n < end {
            for k in 0..8 {
                sep[n + k] = at + m.trailing_zeros() as usize;
                m &= m.wrapping_sub(1);
            }
            n = (n + 8).min(end);
        }
        if n < 128 && at + 64 < d.len() {
            continue;
        }
        let mut i = 0;
        while i + 5 <= n {
            let [p, s, e, st, q] = [sep[i] - 27, sep[i + 1] + 1, sep[i + 2], sep[i + 3], sep[i + 4]];
            let len = e - s;
            // The status and latency follow the endpoint, so 8-byte reads at s are in bounds.
            let (head, tail) = if len >= 8 { (word(d, s), word(d, e - 8)) } else { (word(d, s) & ((1 << (8 * len)) - 1), 0) };
            let id = c.t.id(&d[s..e], d.get(s..s + 64).map(|w| w.try_into().unwrap()), head, tail);
            let row = &mut c.groups[hour(d, p)];
            if id >= row.len() {
                row.resize(c.t.names.len(), Group::default());
            }
            let g = &mut row[id];
            let k = g.n as usize % B;
            if k == 0 {
                c.prev.push(g.block);
                g.block = c.lats.len() as u32;
                c.lats.push([0; B]);
            }
            c.lats[g.block as usize][k] = latency(d, st + 1, q);
            g.n += 1;
            g.errors += (d[e + 1] == b'5') as u32;
            i += 5;
        }
        sep.copy_within(i..n, 0);
        n -= i;
    }
    c
}

// Appends x in decimal; much cheaper than `write!`.
fn num(o: &mut Vec<u8>, mut x: usize) {
    let (mut b, mut i) = ([0; 20], 20);
    loop {
        i -= 1;
        b[i] = b'0' + (x % 10) as u8;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    o.extend_from_slice(&b[i..]);
}

fn hour_label(h: usize) -> String {
    let t = h + 18;
    let (month, day) = match t / 24 {
        0 => (10, 30),
        1 => (10, 31),
        n => (11, n - 1),
    };
    format!("2026-{month:02}-{day:02}T{:02}:00:00Z", t % 24)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = args.get(1).map_or("/data", |s| s.as_str());
    let out_dir = args.get(2).map_or("/out", |s| s.as_str());
    let d = load(&format!("{data_dir}/requests.log"));
    let nt = thread::available_parallelism().map_or(1, |n| n.get());

    // 1. Split at line starts and parse every chunk in parallel.
    let mut bounds = vec![0];
    for i in 1..nt {
        let mut x = (d.len() * i / nt).max(bounds[i - 1]);
        while x > 0 && x < d.len() && d[x - 1] != b'\n' {
            x += 1;
        }
        bounds.push(x);
    }
    bounds.push(d.len());
    let chunks: Vec<Chunk> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let part = &d[bounds[i]..bounds[i + 1]];
            sc.spawn(move || parse(part))
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 2. Rank endpoints in byte order, and map each rank to every chunk's own id.
    let mut all = Names::new();
    let global: Vec<Vec<usize>> = chunks.iter().map(|c| c.t.names.iter().map(|n| all.id_of(n)).collect()).collect();
    let mut order: Vec<usize> = (0..all.names.len()).collect();
    order.sort_unstable_by_key(|&i| &all.names[i]);
    let mut rank = vec![0; order.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    let local: Vec<Vec<usize>> = global.iter().map(|g| {
        let mut l = vec![usize::MAX; order.len()];
        for (id, &i) in g.iter().enumerate() {
            l[rank[i]] = id;
        }
        l
    }).collect();
    let names: Vec<&[u8]> = order.iter().map(|&i| &all.names[i][..]).collect();

    // 3. Each thread takes the next hour, merges every group's blocks from all chunks and
    // finds the nearest-rank percentiles by selection. It writes the hour itself once the
    // hour before is written, so each thread holds one hour of output, not a backlog.
    let next = AtomicUsize::new(0);
    let f = std::fs::File::create(format!("{out_dir}/summary.txt")).unwrap();
    let turn = (Mutex::new(0), Condvar::new()); // the next hour to write
    thread::scope(|sc| {
        for _ in 0..nt {
            let (chunks, local, names, next, f, turn) = (&chunks, &local, &names, &next, &f, &turn);
            sc.spawn(move || {
                let (mut v, mut o) = (Vec::new(), Vec::new());
                loop {
                    let h = next.fetch_add(1, Ordering::Relaxed);
                    if h >= HOURS {
                        return;
                    }
                    let label = hour_label(h);
                    o.clear();
                    for (r, name) in names.iter().enumerate() {
                        v.clear();
                        let mut e = 0;
                        for (c, l) in chunks.iter().zip(local) {
                            if let Some(&g) = c.groups[h].get(l[r]).filter(|g| g.n > 0) {
                                c.gather(g, &mut v);
                                e += g.errors as usize;
                            }
                        }
                        let n = v.len();
                        if n == 0 {
                            continue;
                        }
                        let (mut lo, mut ps) = (0, [0u32; 3]);
                        for (j, p) in [50, 95, 99].into_iter().enumerate() {
                            let rank = (p * n + 99) / 100;
                            ps[j] = if rank > lo { *v[lo..].select_nth_unstable(rank - 1 - lo).1 } else { ps[j - 1] };
                            lo = rank;
                        }
                        o.extend_from_slice(label.as_bytes());
                        o.push(b' ');
                        o.extend_from_slice(name);
                        for x in [n, ps[0] as usize, ps[1] as usize, ps[2] as usize, e, e * 1_000_000 / n] {
                            o.push(b' ');
                            num(&mut o, x);
                        }
                        o.push(b'\n');
                    }
                    let mut w = turn.1.wait_while(turn.0.lock().unwrap(), |w| *w != h).unwrap();
                    (&*f).write_all(&o).unwrap();
                    *w += 1;
                    turn.1.notify_all();
                }
            });
        }
    });
}
