// Speed Demon 2026, Track A (Ingest). See README.md for the approach.
use std::io::Write;
use std::thread;

const HOURS: usize = 72; // 2026-10-30T18:00Z .. 2026-11-02T17:59Z, fixed by the spec

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

fn word(d: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(d[i..i + 8].try_into().unwrap())
}

// Index of the first `b` at or after `i`, 8 bytes at a time.
fn find(d: &[u8], mut i: usize, b: u8) -> usize {
    while i + 8 <= d.len() {
        let w = word(d, i) ^ (b as u64 * 0x0101_0101_0101_0101);
        let z = w.wrapping_sub(0x0101_0101_0101_0101) & !w & 0x8080_8080_8080_8080;
        if z != 0 {
            return i + z.trailing_zeros() as usize / 8;
        }
        i += 8;
    }
    i + d[i..].iter().position(|&c| c == b).unwrap()
}

// The latency that ends just before the newline at q. Digits have bit 4 set and a space
// does not, so the last space in the 8 bytes before q marks where the number starts.
fn latency(d: &[u8], q: usize) -> u32 {
    let w = word(d, q - 8);
    let other = !w & 0x1010_1010_1010_1010;
    if other == 0 {
        // 8 to 10 digits: rare, so go byte by byte.
        let j = (q - 11..q - 8).rev().find(|&i| d[i] == b' ').unwrap() + 1;
        return d[j..q].iter().fold(0, |lat, &b| lat * 10 + (b - b'0') as u32);
    }
    // Keep the digit bytes above the last non-digit (byte k has its flag at bit 8k + 4);
    // the zeroed bytes below read as leading zeros.
    let mut v = w & (u64::MAX << (67 - other.leading_zeros()));
    v = (v & 0x0f0f_0f0f_0f0f_0f0f).wrapping_mul(2561) >> 8;
    v = (v & 0x00ff_00ff_00ff_00ff).wrapping_mul(6_553_601) >> 16;
    ((v & 0x0000_ffff_0000_ffff).wrapping_mul(42_949_672_960_001) >> 32) as u32
}

fn hour(d: &[u8], p: usize) -> usize {
    let digit = |i: usize| (d[i] - b'0') as usize;
    let day = digit(p + 8) * 10 + digit(p + 9);
    (if day >= 30 { day - 30 } else { day + 1 }) * 24 + digit(p + 11) * 10 + digit(p + 12) - 18
}

// One thread's view of its chunk, with its own endpoint ids.
struct Chunk<'a> {
    d: &'a [u8],
    ids: Vec<u16>,           // endpoint id of every line
    tally: Vec<[u32; 2]>,    // [count, 5xx count] at id * HOURS + hour
    names: Vec<&'a [u8]>,
    slots: Vec<Slot>, // open addressing; id 0 = empty
}

// An endpoint's first and last 8 bytes and its length; they are the whole name up to 16 bytes.
#[derive(Clone, Copy, Default)]
struct Slot {
    head: u64,
    tail: u64,
    len: u32,
    id: u32, // id + 1
}

impl<'a> Chunk<'a> {
    fn id(&mut self, ep: &'a [u8], head: u64, tail: u64) -> usize {
        let len = ep.len();
        let h = (head ^ tail.rotate_left(29) ^ len as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        let mask = self.slots.len() - 1;
        let mut i = (h >> 40) as usize & mask;
        loop {
            let s = self.slots[i];
            if s.id == 0 {
                self.names.push(ep);
                self.tally.resize(self.names.len() * HOURS, [0; 2]);
                self.slots[i] = Slot { head, tail, len: len as u32, id: self.names.len() as u32 };
                assert!(self.names.len() < self.slots.len() / 2);
                return self.names.len() - 1;
            }
            if s.head == head && s.tail == tail && s.len as usize == len
                && (len <= 16 || ep[8..len - 8] == self.names[s.id as usize - 1][8..len - 8])
            {
                return s.id as usize - 1;
            }
            i = (i + 1) & mask;
        }
    }
}

// Pass 1: intern each line's endpoint and count lines and 5xx per (endpoint, hour).
fn parse(d: &[u8]) -> Chunk<'_> {
    let mut c = Chunk { d, ids: Vec::with_capacity(d.len() / 60), tally: Vec::new(), names: Vec::new(), slots: vec![Slot::default(); 1 << 13] };
    let mut p = 0;
    while p < d.len() {
        // The method is 3 to 7 letters from p + 28, so its end is in the word at p + 31.
        let s = find(d, p + 31, b' ') + 1;
        // At least " 100 0\n" follows the endpoint's first byte, so 8-byte reads at s are in bounds.
        let e = find(d, s, b' ');
        let len = e - s;
        let (head, tail) = if len >= 8 { (word(d, s), word(d, e - 8)) } else { (word(d, s) & ((1 << (8 * len)) - 1), 0) };
        let id = c.id(&d[s..e], head, tail);
        let t = &mut c.tally[id * HOURS + hour(d, p)];
        t[0] += 1;
        t[1] += (d[e + 1] == b'5') as u32;
        c.ids.push(id as u16);
        p = find(d, e + 6, b'\n') + 1;
    }
    assert!(c.names.len() <= 1 << 16);
    c
}

#[derive(Clone, Copy)]
struct Shared(*mut u32);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}
impl Shared {
    fn at(self, i: usize) -> *mut u32 {
        unsafe { self.0.add(i) }
    }
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

    // 1. Split at line starts and parse and count every chunk in parallel.
    let mut bounds = vec![0];
    for i in 1..nt {
        let mut x = (d.len() * i / nt).max(bounds[i - 1]);
        while x > 0 && x < d.len() && d[x - 1] != b'\n' {
            x += 1;
        }
        bounds.push(x);
    }
    bounds.push(d.len());
    let mut chunks: Vec<Chunk> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let part = &d[bounds[i]..bounds[i + 1]];
            sc.spawn(move || parse(part))
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 2. Rank endpoints in byte order. Latencies are stored by key = rank * HOURS + hour.
    let mut names: Vec<&[u8]> = chunks.iter().flat_map(|c| c.names.iter().copied()).collect();
    names.sort_unstable();
    names.dedup();
    let ne = names.len();
    let nk = ne * HOURS;
    let local: Vec<Vec<usize>> = chunks.iter().map(|c| {
        let mut l = vec![usize::MAX; ne];
        for (id, n) in c.names.iter().enumerate() {
            l[names.binary_search(n).unwrap()] = id;
        }
        l
    }).collect();

    // 3. Turn each thread's counts into its write offsets in one key-ordered latency array.
    let mut start = vec![0usize; nk + 1];
    let mut errors = vec![0u32; nk];
    let mut total = 0;
    for r in 0..ne {
        for h in 0..HOURS {
            let k = r * HOURS + h;
            start[k] = total;
            for (c, l) in chunks.iter_mut().zip(&local) {
                if l[r] != usize::MAX {
                    let t = &mut c.tally[l[r] * HOURS + h];
                    errors[k] += t[1];
                    total += t[0] as usize;
                    t[0] = (total - t[0] as usize) as u32;
                }
            }
        }
    }
    start[nk] = total;

    // 4. Scatter latencies; the threads write disjoint slots.
    let mut lats = vec![0u32; total];
    let out = Shared(lats.as_mut_ptr());
    thread::scope(|sc| {
        for c in chunks.iter_mut() {
            sc.spawn(move || {
                // Pass 2: reread each line's hour and latency (read backwards from its end).
                let d = c.d;
                let mut p = 0;
                for &id in &c.ids {
                    // With the shortest method, the latency starts at p + 37 + the endpoint's length.
                    let q = find(d, p + 37 + c.names[id as usize].len(), b'\n');
                    let lat = latency(d, q);
                    let slot = &mut c.tally[id as usize * HOURS + hour(d, p)][0];
                    unsafe { *out.at(*slot as usize) = lat };
                    *slot += 1;
                    p = q + 1;
                }
            });
        }
    });
    drop(chunks);

    // 5. Nearest-rank percentiles by selection, each thread taking a run of hours.
    let hours: Vec<String> = (0..HOURS).map(hour_label).collect();
    let parts: Vec<Vec<u8>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (start, errors, names, hours) = (&start, &errors, &names, &hours);
            sc.spawn(move || {
                let mut o = Vec::new();
                for h in HOURS * i / nt..HOURS * (i + 1) / nt {
                    for r in 0..ne {
                        let k = r * HOURS + h;
                        let n = start[k + 1] - start[k];
                        if n == 0 {
                            continue;
                        }
                        // Groups are disjoint, so the threads touch disjoint slices.
                        let v = unsafe { std::slice::from_raw_parts_mut(out.at(start[k]), n) };
                        let (mut lo, mut ps) = (0, [0u32; 3]);
                        for (j, p) in [50, 95, 99].into_iter().enumerate() {
                            let rank = (p * n + 99) / 100;
                            ps[j] = if rank > lo { *v[lo..].select_nth_unstable(rank - 1 - lo).1 } else { ps[j - 1] };
                            lo = rank;
                        }
                        let e = errors[k] as usize;
                        o.extend_from_slice(hours[h].as_bytes());
                        o.push(b' ');
                        o.extend_from_slice(names[r]);
                        writeln!(o, " {n} {} {} {} {e} {}", ps[0], ps[1], ps[2], e * 1_000_000 / n).unwrap();
                    }
                }
                o
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut f = std::fs::File::create(format!("{out_dir}/summary.txt")).unwrap();
    for p in &parts {
        f.write_all(p).unwrap();
    }
}
