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
    // PROT_READ, MAP_PRIVATE | MAP_POPULATE
    let p = unsafe { mmap(std::ptr::null_mut(), len, 1, 0x2 | 0x8000, f.as_raw_fd(), 0) };
    assert!(p as isize != -1, "mmap failed");
    unsafe { std::slice::from_raw_parts(p, len) }
}

#[cfg(not(unix))]
fn load(path: &str) -> &'static [u8] {
    std::fs::read(path).unwrap().leak()
}

// One thread's view of its chunk: records plus its own endpoint table.
struct Chunk<'a> {
    recs: Vec<u64>, // endpoint id << 40 | is_5xx << 39 | hour << 32 | latency
    names: Vec<&'a [u8]>,
    slots: Vec<u32>, // open addressing, id + 1, 0 = empty
}

impl<'a> Chunk<'a> {
    fn id(&mut self, ep: &'a [u8]) -> u64 {
        let mut h = ep.len() as u64;
        for &b in ep {
            h = (h.rotate_left(5) ^ b as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
        let mask = self.slots.len() - 1;
        let mut i = (h >> 40) as usize & mask;
        loop {
            match self.slots[i] {
                0 => {
                    self.names.push(ep);
                    self.slots[i] = self.names.len() as u32;
                    return self.names.len() as u64 - 1;
                }
                s if self.names[s as usize - 1] == ep => return s as u64 - 1,
                _ => i = (i + 1) & mask,
            }
        }
    }
}

fn parse(d: &[u8]) -> Chunk<'_> {
    let mut c = Chunk { recs: Vec::with_capacity(d.len() / 70), names: Vec::new(), slots: vec![0; 1 << 14] };
    let digit = |i: usize| (d[i] - b'0') as u64;
    let mut p = 0;
    while p < d.len() {
        let day = digit(p + 8) * 10 + digit(p + 9);
        let hour = (if day >= 30 { day - 30 } else { day + 1 }) * 24 +digit(p + 11) * 10 + digit(p + 12) - 18;
        // Skip the method by its first letters: GET DELETE HEAD OPTIONS POST PUT PATCH.
        let s = p + match (d[p + 28], d[p + 29]) {
            (b'G', _) | (b'P', b'U') => 32,
            (b'H', _) | (b'P', b'O') => 33,
            (b'P', _) => 34,
            (b'D', _) => 35,
            _ => 36,
        };
        let mut e = s + 1;
        while d[e] != b' ' {
            e += 1;
        }
        let id = c.id(&d[s..e]);
        let err = (d[e + 1] == b'5') as u64;
        let mut q = e + 5;
        let mut lat = 0;
        while d[q] != b'\n' {
            lat = lat * 10 + digit(q);
            q += 1;
        }
        c.recs.push(id << 40 | err << 39 | hour << 32 | lat);
        p = q + 1;
    }
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

    // 2. Rank endpoints in byte order, so key = hour * ne + rank is the output order.
    let mut names: Vec<&[u8]> = chunks.iter().flat_map(|c| c.names.iter().copied()).collect();
    names.sort_unstable();
    names.dedup();
    let ne = names.len();
    let nk = HOURS * ne;
    let ranks: Vec<Vec<usize>> = chunks.iter()
        .map(|c| c.names.iter().map(|n| names.binary_search(n).unwrap()).collect())
        .collect();
    let key = |r: u64, rank: &[usize]| (r >> 32 & 0x7f) as usize * ne + rank[(r >> 40) as usize];

    // 3. Per-thread [count, errors] per key.
    let mut tallies: Vec<Vec<[u32; 2]>> = thread::scope(|sc| {
        let hs: Vec<_> = chunks.iter().zip(&ranks).map(|(c, rank)| sc.spawn(move || {
            let mut t = vec![[0u32; 2]; nk];
            for &r in &c.recs {
                let k = &mut t[key(r, rank)];
                k[0] += 1;
                k[1] += (r >> 39 & 1) as u32;
            }
            t
        })).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 4. Turn counts into each thread's write offset in one key-ordered latency array.
    let mut start = vec![0usize; nk + 1];
    let mut errors = vec![0u32; nk];
    let mut total = 0;
    for k in 0..nk {
        start[k] = total;
        for t in tallies.iter_mut() {
            errors[k] += t[k][1];
            total += t[k][0] as usize;
            t[k][0] = (total - t[k][0] as usize) as u32;
        }
    }
    start[nk] = total;

    // 5. Scatter latencies; the threads write disjoint slots.
    let mut lats = vec![0u32; total];
    let out = Shared(lats.as_mut_ptr());
    thread::scope(|sc| {
        for ((c, rank), t) in chunks.iter().zip(&ranks).zip(tallies.iter_mut()) {
            sc.spawn(move || {
                for &r in &c.recs {
                    let slot = &mut t[key(r, rank)][0];
                    unsafe { *out.at(*slot as usize) = r as u32 };
                    *slot += 1;
                }
            });
        }
    });
    drop(chunks);
    drop(tallies);

    // 6. Nearest-rank percentiles by selection, over key ranges of roughly equal line counts.
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
