// Speed Demon 2026, Track D (Traverse). See README.md for the approach.
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::Relaxed};
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

// The same graph with its nodes renamed in the order the threads' orders list them, one after
// another, each thread handling its own order: first the new offsets and rank (old id to new id),
// then the edges, renamed. Also counts the renamed graph's in-degrees into ins, for the transpose.
fn renumber(g: Graph, orders: &[Vec<u32>], ins: &[AtomicU32]) -> (Graph, Vec<u32>) {
    let n = g.off.len() - 1;
    let base: Vec<usize> = orders.iter().scan(0, |acc, o| { let b = *acc; *acc += o.len(); Some(b) }).collect();
    let mut off = vec![0u32; n + 1];
    let mut rank = vec![0u32; n];
    let (fo, ro) = (Shared(off.as_mut_ptr()), Shared(rank.as_mut_ptr()));
    let totals: Vec<u32> = thread::scope(|sc| {
        let hs: Vec<_> = orders.iter().zip(&base).map(|(order, &b)| {
            let g = &g;
            sc.spawn(move || {
                let mut total = 0;
                for (j, &u) in order.iter().enumerate() {
                    unsafe {
                        *fo.at(b + j) = total;
                        *ro.at(u as usize) = (b + j) as u32;
                    }
                    total += g.off[u as usize + 1] - g.off[u as usize];
                }
                total
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let ebase: Vec<u32> = totals.iter().scan(0, |acc, &t| { let b = *acc; *acc += t; Some(b) }).collect();
    let m = totals.iter().sum::<u32>();
    off[n] = m;
    let mut adj = vec![[0u32; 2]; m as usize];
    let out = Shared(adj.as_mut_ptr());
    thread::scope(|sc| {
        for ((order, &b), &e) in orders.iter().zip(&base).zip(&ebase) {
            let (g, rank) = (&g, &rank);
            sc.spawn(move || {
                for (j, &u) in order.iter().enumerate() {
                    let at = unsafe { &mut *fo.at(b + j) };
                    *at += e;
                    let u = u as usize;
                    for (k, &[v, w]) in g.adj[g.off[u] as usize..g.off[u + 1] as usize].iter().enumerate() {
                        let y = rank[v as usize];
                        ins[y as usize].fetch_add(1, Relaxed);
                        unsafe { *out.at(*at as usize + k) = [y, w] };
                    }
                }
            });
        }
    });
    (Graph { off, adj }, rank)
}

// The reverse graph of g, given its in-degrees: place every edge u -> v as v <- u. Once nodes are
// renumbered for locality, u and v are mostly close, so the counters and the writes stay local.
fn transpose(g: &Graph, ins: &[AtomicU32], nt: usize) -> Graph {
    let n = g.off.len() - 1;
    let each = |f: &(dyn Fn(usize, u32, u32) + Sync)| thread::scope(|sc| {
        for i in 0..nt {
            sc.spawn(move || {
                for u in n * i / nt..n * (i + 1) / nt {
                    for &[v, w] in &g.adj[g.off[u] as usize..g.off[u + 1] as usize] {
                        f(u, v, w);
                    }
                }
            });
        }
    });
    let off = offsets(ins);
    let mut adj = vec![[0u32; 2]; g.adj.len()];
    let out = Shared(adj.as_mut_ptr());
    each(&|u, v, w| unsafe { *out.at(ins[v as usize].fetch_add(1, Relaxed) as usize) = [u as u32, w] });
    Graph { off, adj }
}

// Distances of one search, as a paged array: node ids are renumbered for locality, so a search
// touches few blocks of 1,024 consecutive ids, and only those get a page. Lookups cost what an array
// costs, and memory what the search touches.
const PAGE: usize = 1024;

struct Map<V> {
    dir: Vec<u32>, // page index + 1 for each block of ids, 0 = none
    pages: Vec<[V; PAGE]>,
    used: usize,
    blocks: Vec<u32>,
}

// A stored distance; NONE marks a node not reached.
trait Dist: Copy + Eq {
    const NONE: Self;
}
impl Dist for u32 {
    const NONE: u32 = u32::MAX;
}
impl Dist for u64 {
    const NONE: u64 = u64::MAX;
}

impl<V: Dist> Map<V> {
    fn new(n: usize) -> Self {
        Map { dir: vec![0; n.div_ceil(PAGE)], pages: Vec::new(), used: 0, blocks: Vec::new() }
    }

    fn get(&self, k: u32) -> V {
        match self.dir[k as usize / PAGE] {
            0 => V::NONE,
            p => self.pages[p as usize - 1][k as usize % PAGE],
        }
    }

    fn set(&mut self, k: u32, v: V) {
        let b = k as usize / PAGE;
        if self.dir[b] == 0 {
            if self.used == self.pages.len() {
                self.pages.push([V::NONE; PAGE]);
            } else {
                self.pages[self.used] = [V::NONE; PAGE];
            }
            self.used += 1;
            self.dir[b] = self.used as u32;
            self.blocks.push(b as u32);
        }
        self.pages[self.dir[b] as usize - 1][k as usize % PAGE] = v;
    }

    // Empty the map; pages a long search needed beyond 64 are given back.
    fn clear(&mut self) {
        for b in self.blocks.drain(..) {
            self.dir[b as usize] = 0;
        }
        self.used = 0;
        self.pages.truncate(64);
    }
}

// A heap entry: a tentative distance and its node, ordered by distance, and how distances are
// stored. Most searches use one u64, distance << 21 | node, which is half the size and compares in
// one instruction, and store distances as u32, halving the pages a search touches; a search whose
// distances reach 2^32 - 1 is run again with plain pairs and u64 distances.
trait Key: Ord + Copy {
    type D: Dist;
    fn make(d: u64, u: u32) -> Option<Self>;
    fn split(self) -> (u64, u32);
    fn store(d: u64) -> Self::D;
    fn load(d: Self::D) -> u64; // u64::MAX for NONE
}

impl Key for u64 {
    type D = u32;
    fn make(d: u64, u: u32) -> Option<u64> {
        (d < u32::MAX as u64).then_some(d << 21 | u as u64)
    }
    fn store(d: u64) -> u32 {
        d as u32
    }
    fn load(d: u32) -> u64 {
        if d == u32::MAX { u64::MAX } else { d as u64 }
    }
    fn split(self) -> (u64, u32) {
        (self >> 21, self as u32 & 0x1f_ffff)
    }
}

impl Key for (u64, u32) {
    type D = u64;
    fn make(d: u64, u: u32) -> Option<Self> {
        Some((d, u))
    }
    fn store(d: u64) -> u64 {
        d
    }
    fn load(d: u64) -> u64 {
        d
    }
    fn split(self) -> (u64, u32) {
        self
    }
}

// One direction of a bidirectional Dijkstra search.
struct Side<'g, K: Key> {
    g: &'g Graph,
    dist: Map<K::D>,
    heap: BinaryHeap<Reverse<K>>,
}

impl<'g, K: Key> Side<'g, K> {
    fn new(g: &'g Graph) -> Self {
        Side { g, dist: Map::new(g.off.len() - 1), heap: BinaryHeap::new() }
    }

    fn reset(&mut self, from: u32) {
        self.dist.clear();
        self.heap.clear();
        self.dist.set(from, K::store(0));
        self.heap.push(Reverse(K::make(0, from).unwrap()));
    }

    // The smallest tentative distance still queued, dropping stale entries; MAX when empty.
    fn top(&mut self) -> u64 {
        while let Some(&Reverse(k)) = self.heap.peek() {
            let (d, u) = k.split();
            if d == K::load(self.dist.get(u)) {
                return d;
            }
            self.heap.pop();
        }
        u64::MAX
    }

    // Settle the top node and relax its edges; any node the other side has reached closes a path.
    // False when a distance does not fit K.
    fn step(&mut self, other: &Self, best: &mut u64) -> bool {
        let (d, u) = self.heap.pop().unwrap().0.split();
        let g = self.g;
        for &[v, w] in &g.adj[g.off[u as usize] as usize..g.off[u as usize + 1] as usize] {
            let nd = d + w as u64;
            if nd < K::load(self.dist.get(v)) {
                let Some(k) = K::make(nd, v) else { return false };
                self.dist.set(v, K::store(nd));
                self.heap.push(Reverse(k));
                let o = K::load(other.dist.get(v));
                if o != u64::MAX {
                    *best = (*best).min(nd + o);
                }
            }
        }
        true
    }
}

// Stop once the two frontiers together cannot beat the best path found; an empty side tops at MAX.
// None when a distance does not fit K.
fn query<'g, K: Key>(f: &mut Side<'g, K>, b: &mut Side<'g, K>, s: u32, t: u32) -> Option<u64> {
    if s == t {
        return Some(0);
    }
    f.reset(s);
    b.reset(t);
    let mut best = u64::MAX;
    loop {
        let (tf, tb) = (f.top(), b.top());
        if tf.saturating_add(tb) >= best {
            return Some(best);
        }
        // Grow the smaller frontier: a side that is about to run dry (an island, a dead end)
        // finishes first instead of letting the other side flood the graph.
        let ok = if f.heap.len() <= b.heap.len() { f.step(b, &mut best) } else { b.step(f, &mut best) };
        if !ok {
            return None;
        }
    }
}

fn main() {
    let t0 = std::time::Instant::now();
    let mut marks = String::new();
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

    // 1. Parse each chunk once, sending every edge to the thread that owns its tail: thread t owns
    //    nodes lo(t)..lo(t + 1).
    let lo = |t: usize| (n * t).div_ceil(nt);
    let owner = |u: usize| u * nt / n;
    let buckets: Vec<Vec<Vec<[u32; 3]>>> = thread::scope(|sc| {
        let hs: Vec<_> = chunks.iter().map(|c| sc.spawn(move || {
            let mut out: Vec<Vec<[u32; 3]>> = (0..nt).map(|_| Vec::with_capacity(c.len() / 18 / nt + 64)).collect();
            each_edge(c, |u, v, w| out[owner(u)].push([u as u32, v as u32, w]));
            out
        })).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    marks += &format!("_0x{}", t0.elapsed().as_micros());
    // 2. Each owner counts its nodes' out-degrees and places their edges in the forward graph; only
    //    the start of each owner's range in adj needs the others' totals.
    let mut off = vec![0u32; n + 1];
    let fo = Shared(off.as_mut_ptr());
    let totals: Vec<u32> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|t| {
            let buckets = &buckets;
            sc.spawn(move || {
                let (a, b) = (lo(t), lo(t + 1));
                let off = unsafe { std::slice::from_raw_parts_mut(fo.at(a), b - a) };
                for e in buckets.iter().flat_map(|out| &out[t]) {
                    off[e[0] as usize - a] += 1;
                }
                let mut total = 0;
                for o in off.iter_mut() {
                    (*o, total) = (total, total + *o);
                }
                total
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let base: Vec<u32> = totals.iter().scan(0, |acc, &x| { let b = *acc; *acc += x; Some(b) }).collect();
    let m = totals.iter().sum::<u32>();
    off[n] = m;
    let mut adj = vec![[0u32; 2]; m as usize];
    let fa = Shared(adj.as_mut_ptr());
    thread::scope(|sc| {
        for t in 0..nt {
            let (buckets, base) = (&buckets, base[t]);
            sc.spawn(move || {
                let (a, b) = (lo(t), lo(t + 1));
                let off = unsafe { std::slice::from_raw_parts_mut(fo.at(a), b - a) };
                let mut next: Vec<u32> = off.to_vec();
                for e in buckets.iter().flat_map(|out| &out[t]) {
                    let x = &mut next[e[0] as usize - a];
                    unsafe { *fa.at((base + *x) as usize) = [e[1], e[2]] };
                    *x += 1;
                }
                for o in off.iter_mut() {
                    *o += base;
                }
            });
        }
    });
    drop(buckets);
    let fwd = Graph { off, adj };

    marks += &format!("_1x{}", t0.elapsed().as_micros());
    // 3. Renumber nodes so a search walks nearby memory: every thread grows breadth-first regions
    //    from seeds in its own id range, claiming nodes with an atomic flag, and the threads'
    //    visiting orders, one after another, are the new numbering.
    let claimed: Vec<AtomicBool> = (0..n).map(|_| AtomicBool::new(false)).collect();
    let orders: Vec<Vec<u32>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (fwd, claimed) = (&fwd, &claimed);
            sc.spawn(move || {
                let mut order = Vec::new();
                for root in n * i / nt..n * (i + 1) / nt {
                    if claimed[root].swap(true, Relaxed) {
                        continue;
                    }
                    let mut h = order.len();
                    order.push(root as u32);
                    while h < order.len() {
                        let u = order[h] as usize;
                        h += 1;
                        for &[v, _] in &fwd.adj[fwd.off[u] as usize..fwd.off[u + 1] as usize] {
                            if !claimed[v as usize].load(Relaxed) && !claimed[v as usize].swap(true, Relaxed) {
                                order.push(v);
                            }
                        }
                    }
                }
                order
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    drop(claimed);
    marks += &format!("_2x{}", t0.elapsed().as_micros());
    let ins: Vec<AtomicU32> = (0..=n).map(|_| AtomicU32::new(0)).collect();
    let (fwd, rank) = renumber(fwd, &orders, &ins);
    drop(orders);
    marks += &format!("_3x{}", t0.elapsed().as_micros());
    let bwd = transpose(&fwd, &ins, nt);
    drop(ins);

    marks += &format!("_4x{}", t0.elapsed().as_micros());
    // 4. Answer the queries on every core, each thread taking the next unanswered one.
    let q = std::fs::read(format!("{data_dir}/queries.txt")).unwrap();
    let mut pairs = Vec::new();
    let mut p = 0;
    while p < q.len() {
        pairs.push((rank[num(&q, &mut p) as usize], rank[num(&q, &mut p) as usize]));
    }
    drop(rank);
    let narrow = n <= 1 << 21; // node ids fit u64 heap keys
    let next = AtomicUsize::new(0);
    let answers: Vec<Vec<(usize, u64)>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|_| {
            let (fwd, bwd, pairs, next) = (&fwd, &bwd, &pairs, &next);
            sc.spawn(move || {
                let (mut f, mut b) = (Side::<u64>::new(fwd), Side::<u64>::new(bwd));
                let (mut wf, mut wb) = (None, None);
                let mut done = Vec::new();
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(&(s, t)) = pairs.get(i) else { return done };
                    let d = narrow.then(|| query(&mut f, &mut b, s, t)).flatten().unwrap_or_else(|| {
                        let wf = wf.get_or_insert_with(|| Side::<(u64, u32)>::new(fwd));
                        let wb = wb.get_or_insert_with(|| Side::<(u64, u32)>::new(bwd));
                        query(wf, wb, s, t).unwrap()
                    });
                    done.push((i, d));
                }
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    marks += &format!("_5x{}", t0.elapsed().as_micros());
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
    marks += &format!("_ex{}", t0.elapsed().as_micros());
    std::fs::write(format!("{out_dir}/PROFD{marks}"), b"").unwrap();
}
