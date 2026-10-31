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

// The same graph with node order[x] renamed x (rank is the inverse); threads fill disjoint ranges.
fn renumber(g: Graph, order: &[u32], rank: &[u32], nt: usize) -> Graph {
    let mut off = Vec::with_capacity(order.len() + 1);
    let mut total = 0;
    for &u in order {
        off.push(total);
        total += g.off[u as usize + 1] - g.off[u as usize];
    }
    off.push(total);
    let mut adj = vec![[0u32; 2]; total as usize];
    let out = Shared(adj.as_mut_ptr());
    thread::scope(|sc| {
        for i in 0..nt {
            let (g, off) = (&g, &off);
            sc.spawn(move || {
                for x in order.len() * i / nt..order.len() * (i + 1) / nt {
                    let u = order[x] as usize;
                    for (j, &[v, w]) in g.adj[g.off[u] as usize..g.off[u + 1] as usize].iter().enumerate() {
                        unsafe { *out.at(off[x] as usize + j) = [rank[v as usize], w] };
                    }
                }
            });
        }
    });
    Graph { off, adj }
}

// The reverse graph of g: count in-degrees, then place every edge u -> v as v <- u. Once nodes are
// renumbered for locality, u and v are mostly close, so the counters and the writes stay local.
fn transpose(g: &Graph, nt: usize) -> Graph {
    let n = g.off.len() - 1;
    let ins: Vec<AtomicU32> = (0..=n).map(|_| AtomicU32::new(0)).collect();
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
    each(&|_, v, _| {
        ins[v as usize].fetch_add(1, Relaxed);
    });
    let off = offsets(&ins);
    let mut adj = vec![[0u32; 2]; g.adj.len()];
    let out = Shared(adj.as_mut_ptr());
    each(&|u, v, w| unsafe { *out.at(ins[v as usize].fetch_add(1, Relaxed) as usize) = [u as u32, w] });
    Graph { off, adj }
}

// Distances of one search, as a paged array: node ids are renumbered for locality, so a search
// touches few blocks of 1,024 consecutive ids, and only those get a page. Lookups cost what an array
// costs, and memory what the search touches.
const PAGE: usize = 1024;

struct Map {
    dir: Vec<u32>, // page index + 1 for each block of ids, 0 = none
    pages: Vec<[u64; PAGE]>,
    used: usize,
    blocks: Vec<u32>,
}

impl Map {
    fn new(n: usize) -> Map {
        Map { dir: vec![0; n.div_ceil(PAGE)], pages: Vec::new(), used: 0, blocks: Vec::new() }
    }

    fn get(&self, k: u32) -> u64 {
        match self.dir[k as usize / PAGE] {
            0 => u64::MAX,
            p => self.pages[p as usize - 1][k as usize % PAGE],
        }
    }

    fn set(&mut self, k: u32, v: u64) {
        let b = k as usize / PAGE;
        if self.dir[b] == 0 {
            if self.used == self.pages.len() {
                self.pages.push([u64::MAX; PAGE]);
            } else {
                self.pages[self.used] = [u64::MAX; PAGE];
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

    // 1. Count out-degrees in parallel.
    let outs: Vec<AtomicU32> = (0..=n).map(|_| AtomicU32::new(0)).collect();
    thread::scope(|sc| {
        for c in &chunks {
            let outs = &outs;
            sc.spawn(move || each_edge(c, |u, _, _| {
                outs[u].fetch_add(1, Relaxed);
            }));
        }
    });

    // 2. Parse again and place every edge in the forward graph.
    let fo = offsets(&outs);
    let m = fo[n] as usize;
    let mut fwd = Graph { off: fo, adj: vec![[0; 2]; m] };
    let fa = Shared(fwd.adj.as_mut_ptr());
    thread::scope(|sc| {
        for c in &chunks {
            let outs = &outs;
            sc.spawn(move || each_edge(c, |u, v, w| unsafe {
                *fa.at(outs[u].fetch_add(1, Relaxed) as usize) = [v as u32, w];
            }));
        }
    });
    drop(outs);

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
    let order = orders.concat();
    drop(orders);
    let mut rank = vec![0u32; n];
    for (x, &u) in order.iter().enumerate() {
        rank[u as usize] = x as u32;
    }
    let fwd = renumber(fwd, &order, &rank, nt);
    drop(order);
    let bwd = transpose(&fwd, nt);

    // 4. Answer the queries on every core, each thread taking the next unanswered one.
    let q = std::fs::read(format!("{data_dir}/queries.txt")).unwrap();
    let mut pairs = Vec::new();
    let mut p = 0;
    while p < q.len() {
        pairs.push((rank[num(&q, &mut p) as usize], rank[num(&q, &mut p) as usize]));
    }
    drop(rank);
    let next = AtomicUsize::new(0);
    let answers: Vec<Vec<(usize, u64)>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|_| {
            let (fwd, bwd, pairs, next) = (&fwd, &bwd, &pairs, &next);
            sc.spawn(move || {
                let mut f = Side { g: fwd, dist: Map::new(n), heap: BinaryHeap::new() };
                let mut b = Side { g: bwd, dist: Map::new(n), heap: BinaryHeap::new() };
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
