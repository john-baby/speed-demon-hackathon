// Speed Demon 2026, Track D (Traverse). See README.md for the approach.
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::Write;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
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

// Parse the unsigned number at *p and step past the one separator after it.
fn num(d: &[u8], p: &mut usize) -> u64 {
    let mut x = 0;
    while d[*p] >= b'0' {
        x = x * 10 + (d[*p] - b'0') as u64;
        *p += 1;
    }
    *p += 1;
    x
}

// Call f(u, v, w) for every edge line in the chunk. Self-loops never shorten a path, so skip them.
fn each_edge(d: &[u8], mut f: impl FnMut(usize, usize, u32)) {
    let mut p = 0;
    while p < d.len() {
        let (u, v, w) = (num(d, &mut p) as usize, num(d, &mut p) as usize, num(d, &mut p) as u32);
        if u != v {
            f(u, v, w);
        }
    }
}

#[derive(Clone, Copy)]
struct Shared<T>(*mut T);
unsafe impl<T> Send for Shared<T> {}
unsafe impl<T> Sync for Shared<T> {}
impl<T> Shared<T> {
    fn at(self, i: usize) -> *mut T {
        unsafe { self.0.add(i) }
    }
}

// Compressed adjacency: the edges of node u are adj[off[u]..off[u + 1]], each [other end, weight].
struct Graph {
    off: Vec<u32>,
    adj: Vec<[u32; 2]>,
}

// Turn per-node counts into start offsets, leaving each counter at its node's first write slot.
fn offsets(count: &[AtomicU32]) -> Vec<u32> {
    let mut total = 0;
    let off: Vec<u32> = count.iter().map(|c| {
        let start = total;
        total += c.load(Relaxed);
        c.store(start, Relaxed);
        start
    }).collect();
    off
}

// Distances of one search, in a small open-addressing table so a search costs what it touches.
struct Map {
    keys: Vec<u32>,
    vals: Vec<u64>,
    used: Vec<usize>,
    shift: u32,
}

const EMPTY: u32 = u32::MAX;

impl Map {
    fn new(bits: u32) -> Map {
        Map { keys: vec![EMPTY; 1 << bits], vals: vec![0; 1 << bits], used: Vec::new(), shift: 64 - bits }
    }

    fn slot(&self, k: u32) -> usize {
        let mask = self.keys.len() - 1;
        let mut i = ((k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> self.shift) as usize;
        while self.keys[i] != k && self.keys[i] != EMPTY {
            i = (i + 1) & mask;
        }
        i
    }

    fn get(&self, k: u32) -> u64 {
        let i = self.slot(k);
        if self.keys[i] == k { self.vals[i] } else { u64::MAX }
    }

    fn set(&mut self, k: u32, v: u64) {
        let i = self.slot(k);
        if self.keys[i] == EMPTY {
            if self.used.len() * 2 >= self.keys.len() {
                self.grow();
                return self.set(k, v);
            }
            self.keys[i] = k;
            self.used.push(i);
        }
        self.vals[i] = v;
    }

    fn grow(&mut self) {
        let old = std::mem::replace(self, Map::new(65 - self.shift));
        for &i in &old.used {
            self.set(old.keys[i], old.vals[i]);
        }
    }

    fn clear(&mut self) {
        for i in self.used.drain(..) {
            self.keys[i] = EMPTY;
        }
    }
}

// One direction of a bidirectional Dijkstra search.
struct Side<'g> {
    g: &'g Graph,
    dist: Map,
    heap: BinaryHeap<Reverse<(u64, u32)>>,
}

impl<'g> Side<'g> {
    fn reset(&mut self, from: u32) {
        self.dist.clear();
        self.heap.clear();
        self.dist.set(from, 0);
        self.heap.push(Reverse((0, from)));
    }

    // The smallest tentative distance still queued, dropping stale entries; MAX when empty.
    fn top(&mut self) -> u64 {
        while let Some(&Reverse((d, u))) = self.heap.peek() {
            if d == self.dist.get(u) {
                return d;
            }
            self.heap.pop();
        }
        u64::MAX
    }

    // Settle the top node and relax its edges; any node the other side has reached closes a path.
    fn step(&mut self, other: &Side, best: &mut u64) {
        let Reverse((d, u)) = self.heap.pop().unwrap();
        let g = self.g;
        for &[v, w] in &g.adj[g.off[u as usize] as usize..g.off[u as usize + 1] as usize] {
            let nd = d + w as u64;
            if nd < self.dist.get(v) {
                self.dist.set(v, nd);
                self.heap.push(Reverse((nd, v)));
                let o = other.dist.get(v);
                if o != u64::MAX {
                    *best = (*best).min(nd + o);
                }
            }
        }
    }
}

// Stop once the two frontiers together cannot beat the best path found; an empty side tops at MAX.
fn query(f: &mut Side, b: &mut Side, s: u32, t: u32) -> u64 {
    if s == t {
        return 0;
    }
    f.reset(s);
    b.reset(t);
    let mut best = u64::MAX;
    loop {
        let (tf, tb) = (f.top(), b.top());
        if tf.saturating_add(tb) >= best {
            return best;
        }
        // Grow the smaller frontier: a side that is about to run dry (an island, a dead end)
        // finishes first instead of letting the other side flood the graph.
        if f.heap.len() <= b.heap.len() {
            f.step(b, &mut best);
        } else {
            b.step(f, &mut best);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = args.get(1).map_or("/data", |s| s.as_str());
    let out_dir = args.get(2).map_or("/out", |s| s.as_str());
    let d = load(&format!("{data_dir}/graph.txt"));
    let nt = thread::available_parallelism().map_or(1, |n| n.get());

    let mut p = 0;
    let n = num(d, &mut p) as usize;
    num(d, &mut p);
    let body = &d[p..];
    let mut bounds = vec![0];
    for i in 1..nt {
        let mut x = (body.len() * i / nt).max(bounds[i - 1]);
        while x > 0 && x < body.len() && body[x - 1] != b'\n' {
            x += 1;
        }
        bounds.push(x);
    }
    bounds.push(body.len());
    let chunks: Vec<&[u8]> = (0..nt).map(|i| &body[bounds[i]..bounds[i + 1]]).collect();

    // 1. Count out- and in-degrees in parallel.
    let outs: Vec<AtomicU32> = (0..=n).map(|_| AtomicU32::new(0)).collect();
    let ins: Vec<AtomicU32> = (0..=n).map(|_| AtomicU32::new(0)).collect();
    thread::scope(|sc| {
        for c in &chunks {
            let (outs, ins) = (&outs, &ins);
            sc.spawn(move || each_edge(c, |u, v, _| {
                outs[u].fetch_add(1, Relaxed);
                ins[v].fetch_add(1, Relaxed);
            }));
        }
    });

    // 2. Parse again and place every edge in the forward and the reverse graph.
    let (fo, bo) = (offsets(&outs), offsets(&ins));
    let m = fo[n] as usize;
    let mut fwd = Graph { off: fo, adj: vec![[0; 2]; m] };
    let mut bwd = Graph { off: bo, adj: vec![[0; 2]; m] };
    let (fa, ba) = (Shared(fwd.adj.as_mut_ptr()), Shared(bwd.adj.as_mut_ptr()));
    thread::scope(|sc| {
        for c in &chunks {
            let (outs, ins) = (&outs, &ins);
            sc.spawn(move || each_edge(c, |u, v, w| unsafe {
                *fa.at(outs[u].fetch_add(1, Relaxed) as usize) = [v as u32, w];
                *ba.at(ins[v].fetch_add(1, Relaxed) as usize) = [u as u32, w];
            }));
        }
    });
    drop((outs, ins));

    // 3. Answer the queries on every core, each thread taking the next unanswered one.
    let q = std::fs::read(format!("{data_dir}/queries.txt")).unwrap();
    let mut pairs = Vec::new();
    let mut p = 0;
    while p < q.len() {
        pairs.push((num(&q, &mut p) as u32, num(&q, &mut p) as u32));
    }
    let next = AtomicUsize::new(0);
    let answers: Vec<Vec<(usize, u64)>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|_| {
            let (fwd, bwd, pairs, next) = (&fwd, &bwd, &pairs, &next);
            sc.spawn(move || {
                let mut f = Side { g: fwd, dist: Map::new(16), heap: BinaryHeap::new() };
                let mut b = Side { g: bwd, dist: Map::new(16), heap: BinaryHeap::new() };
                let mut done = Vec::new();
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(&(s, t)) = pairs.get(i) else { return done };
                    done.push((i, query(&mut f, &mut b, s, t)));
                }
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut dist = vec![0; pairs.len()];
    for (i, a) in answers.into_iter().flatten() {
        dist[i] = a;
    }
    let mut o = Vec::new();
    for a in dist {
        if a == u64::MAX {
            o.extend_from_slice(b"-1\n");
        } else {
            writeln!(o, "{a}").unwrap();
        }
    }
    std::fs::write(format!("{out_dir}/distances.txt"), o).unwrap();
}
