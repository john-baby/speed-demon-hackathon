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
    slots: Vec<u32>,         // open addressing, id + 1, 0 = empty
}

impl<'a> Chunk<'a> {
    fn id(&mut self, ep: &'a [u8], h: u64) -> usize {
        let mask = self.slots.len() - 1;
        let mut i = (h >> 40) as usize & mask;
        loop {
            match self.slots[i] {
                0 => {
                    self.names.push(ep);
                    self.tally.resize(self.names.len() * HOURS, [0; 2]);
                    self.slots[i] = self.names.len() as u32;
                    return self.names.len() - 1;
                }
                s if self.names[s as usize - 1] == ep => return s as usize - 1,
                _ => i = (i + 1) & mask,
            }
        }
    }
}

// Pass 1: intern each line's endpoint and count lines and 5xx per (endpoint, hour).
fn parse(d: &[u8]) -> Chunk<'_> {
    let mut c = Chunk { d, ids: Vec::with_capacity(d.len() / 60), tally: Vec::new(), names: Vec::new(), slots: vec![0; 1 << 14] };
    let mut p = 0;
    while p < d.len() {
        // Skip the method by its first letters: GET DELETE HEAD OPTIONS POST PUT PATCH.
        let s = p + match (d[p + 28], d[p + 29]) {
            (b'G', _) | (b'P', b'U') => 32,
            (b'H', _) | (b'P', b'O') => 33,
            (b'P', _) => 34,
            (b'D', _) => 35,
            _ => 36,
        };
        // At least " 100 0\n" follows the endpoint's first byte, so 8-byte reads at s are in bounds.
        let e = find(d, s, b' ');
        let len = e - s;
        let (head, tail) = if len >= 8 { (word(d, s), word(d, e - 8)) } else { (word(d, s) & ((1 << (8 * len)) - 1), 0) };
        let h = (head ^ tail.rotate_left(29) ^ len as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        let id = c.id(&d[s..e], h);
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

    // 2. Rank endpoints in byte order, so key = hour * ne + rank is the output order.
    let mut names: Vec<&[u8]> = chunks.iter().flat_map(|c| c.names.iter().copied()).collect();
    names.sort_unstable();
    names.dedup();
    let ne = names.len();
    let nk = HOURS * ne;
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
    for k in 0..nk {
        start[k] = total;
        for (c, l) in chunks.iter_mut().zip(&local) {
            if let Some(&id) = l.get(k % ne).filter(|&&id| id != usize::MAX) {
                let t = &mut c.tally[id * HOURS + k / ne];
                errors[k] += t[1];
                total += t[0] as usize;
                t[0] = (total - t[0] as usize) as u32;
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
                    let q = find(d, p + 39, b'\n');
                    let (mut lat, mut m, mut j) = (0u32, 1u32, q - 1);
                    while d[j] != b' ' {
                        lat += (d[j] - b'0') as u32 * m;
                        m = m.wrapping_mul(10);
                        j -= 1;
                    }
                    let slot = &mut c.tally[id as usize * HOURS + hour(d, p)][0];
                    unsafe { *out.at(*slot as usize) = lat };
                    *slot += 1;
                    p = q + 1;
                }
            });
        }
    });
    drop(chunks);

    // 5. Nearest-rank percentiles by selection, over key ranges of roughly equal line counts.
    let mut kb = vec![0];
    for i in 1..nt {
        kb.push(start.partition_point(|&s| s < total * i / nt).min(nk));
    }
    kb.push(nk);
    let hours: Vec<String> = (0..HOURS).map(hour_label).collect();
    let parts: Vec<Vec<u8>> = thread::scope(|sc| {
        let mut rest = &mut lats[..];
        let mut hs = Vec::new();
        for i in 0..nt {
            let (k0, k1) = (kb[i], kb[i + 1]);
            let (mine, tail) = rest.split_at_mut(start[k1] - start[k0]);
            rest = tail;
            let (start, errors, names, hours) = (&start, &errors, &names, &hours);
            hs.push(sc.spawn(move || {
                let mut o = Vec::new();
                for k in k0..k1 {
                    let n = start[k + 1] - start[k];
                    if n == 0 {
                        continue;
                    }
                    let v = &mut mine[start[k] - start[k0]..start[k + 1] - start[k0]];
                    let (mut lo, mut ps) = (0, [0u32; 3]);
                    for (j, p) in [50, 95, 99].into_iter().enumerate() {
                        let r = (p * n + 99) / 100;
                        ps[j] = if r > lo { *v[lo..].select_nth_unstable(r - 1 - lo).1 } else { ps[j - 1] };
                        lo = r;
                    }
                    let e = errors[k] as usize;
                    o.extend_from_slice(hours[k / ne].as_bytes());
                    o.push(b' ');
                    o.extend_from_slice(names[k % ne]);
                    writeln!(o, " {n} {} {} {} {e} {}", ps[0], ps[1], ps[2], e * 1_000_000 / n).unwrap();
                }
                o
            }));
        }
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut f = std::fs::File::create(format!("{out_dir}/summary.txt")).unwrap();
    for p in &parts {
        f.write_all(p).unwrap();
    }
}
