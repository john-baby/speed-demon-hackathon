// Speed Demon 2026, Track B (Index). See README.md for the approach.
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
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

const NONE: u32 = u32::MAX;

// A token of up to 32 bytes, zero-padded; longer tokens can match no query term.
type Key = [u8; 32];

fn hash(k: &Key) -> u64 {
    let w = |i: usize| u64::from_le_bytes(k[i..i + 8].try_into().unwrap());
    let h = (w(0) ^ w(8).rotate_left(17) ^ w(16).rotate_left(31) ^ w(24).rotate_left(47)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^ h >> 29
}

// Open-addressing map from a u64 key (never u64::MAX) to a u32, sized for `n` entries.
struct Map {
    keys: Vec<u64>,
    vals: Vec<u32>,
}

impl Map {
    fn new(n: usize) -> Map {
        let size = (n * 2).next_power_of_two().max(16);
        Map { keys: vec![u64::MAX; size], vals: vec![0; size] }
    }

    fn slot(&self, k: u64, h: u64) -> usize {
        let mask = self.keys.len() - 1;
        let mut i = h as usize & mask;
        while self.keys[i] != k && self.keys[i] != u64::MAX {
            i = (i + 1) & mask;
        }
        i
    }

    fn get(&self, k: u64) -> u32 {
        let i = self.slot(k, k.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 20);
        if self.keys[i] == k { self.vals[i] } else { NONE }
    }

    fn insert(&mut self, k: u64, v: u32) {
        let i = self.slot(k, k.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 20);
        self.keys[i] = k;
        self.vals[i] = v;
    }
}

// The distinct query terms, numbered in order of first appearance.
struct Terms {
    keys: Vec<Key>,
    slots: Vec<u32>, // id + 1, 0 = empty
}

impl Terms {
    fn find(&self, k: &Key) -> (usize, u32) {
        let mask = self.slots.len() - 1;
        let mut i = hash(k) as usize & mask;
        loop {
            match self.slots[i] {
                0 => return (i, NONE),
                s if self.keys[s as usize - 1] == *k => return (i, s - 1),
                _ => i = (i + 1) & mask,
            }
        }
    }

    fn id(&mut self, word: &[u8]) -> u32 {
        let mut k = [0u8; 32];
        k[..word.len()].copy_from_slice(word);
        let (i, id) = self.find(&k);
        if id != NONE {
            return id;
        }
        self.keys.push(k);
        self.slots[i] = self.keys.len() as u32;
        self.keys.len() as u32 - 1
    }
}

enum Query {
    Bool(Vec<(u8, u32)>), // (operator, term); the first operator is ignored
    Phrase(u32),          // the automaton node that ends the phrase
}

// Aho-Corasick over term ids: every phrase is a path from the root; fail[s] is the longest proper
// suffix of s's path that is also a path, and out[s] the nearest node on s's fail chain (s itself
// included) where a phrase ends.
struct Automaton {
    goto: Map, // node << 32 | term -> node
    fail: Vec<u32>,
    out: Vec<u32>,
}

impl Automaton {
    fn step(&self, mut s: u32, t: u32) -> u32 {
        loop {
            let v = self.goto.get((s as u64) << 32 | t as u64);
            if v != NONE {
                return v;
            }
            if s == 0 {
                return 0;
            }
            s = self.fail[s as usize];
        }
    }
}

fn build(phrases: &[Vec<u32>]) -> (Automaton, Vec<u32>) {
    let mut children: Vec<Vec<(u32, u32)>> = vec![vec![]];
    let mut goto = Map::new(phrases.iter().map(|p| p.len()).sum());
    let mut ends = Vec::new();
    for p in phrases {
        let mut s = 0u32;
        for &t in p {
            let key = (s as u64) << 32 | t as u64;
            let mut v = goto.get(key);
            if v == NONE {
                v = children.len() as u32;
                children.push(vec![]);
                children[s as usize].push((t, v));
                goto.insert(key, v);
            }
            s = v;
        }
        ends.push(s);
    }
    let n = children.len();
    let mut end = vec![false; n];
    for &e in &ends {
        end[e as usize] = true;
    }
    let mut a = Automaton { goto, fail: vec![0; n], out: vec![NONE; n] };
    let mut order: Vec<u32> = children[0].iter().map(|&(_, v)| v).collect();
    let mut h = 0;
    while h < order.len() {
        let u = order[h];
        h += 1;
        let fu = a.fail[u as usize];
        a.out[u as usize] = if end[u as usize] { u } else { a.out[fu as usize] };
        for &(t, v) in &children[u as usize] {
            a.fail[v as usize] = a.step(fu, t);
            order.push(v);
        }
    }
    (a, ends)
}

// What one thread finds in its range of documents.
struct Part {
    pairs: Vec<(u32, u32)>, // (bool term, doc), each pair once, docs ascending
    tally: Vec<u32>,        // documents per bool term
    hits: Vec<(u32, u32)>,  // (phrase end node, doc), each pair once
    docs: usize,            // last doc id seen + 1
}

fn scan(d: &[u8], terms: &Terms, in_bool: &[bool], in_phrase: &[bool], ac: &Automaton, nodes: usize) -> Part {
    let mut class = [0u8; 256];
    for b in (b'a'..=b'z').chain(b'0'..=b'9') {
        class[b as usize] = b;
    }
    for b in b'A'..=b'Z' {
        class[b as usize] = b + 32;
    }
    let mut part = Part { pairs: Vec::new(), tally: vec![0; terms.keys.len()], hits: Vec::new(), docs: 0 };
    let mut last = vec![NONE; terms.keys.len()];
    let mut last_hit = vec![NONE; nodes];
    let mut p = 0;
    while p < d.len() {
        let mut doc = 0u32;
        while d[p] != b'\t' {
            doc = doc * 10 + (d[p] - b'0') as u32;
            p += 1;
        }
        p += 1;
        part.docs = doc as usize + 1;
        let mut state = 0;
        let mut key = [0u8; 32];
        let mut len = 0;
        loop {
            let b = d[p];
            p += 1;
            let c = class[b as usize];
            if c != 0 {
                if len < 33 {
                    if len < 32 {
                        key[len] = c;
                    }
                    len += 1;
                }
                continue;
            }
            if len > 0 {
                let t = if len <= 32 { terms.find(&key).1 } else { NONE };
                key = [0; 32];
                len = 0;
                if t != NONE && in_bool[t as usize] && last[t as usize] != doc {
                    last[t as usize] = doc;
                    part.tally[t as usize] += 1;
                    part.pairs.push((t, doc));
                }
                if t == NONE || !in_phrase[t as usize] {
                    state = 0;
                } else {
                    state = ac.step(state, t);
                    let mut x = ac.out[state as usize];
                    while x != NONE {
                        if last_hit[x as usize] != doc {
                            last_hit[x as usize] = doc;
                            part.hits.push((x, doc));
                        }
                        x = ac.out[ac.fail[x as usize] as usize];
                    }
                }
            }
            if b == b'\n' {
                break;
            }
        }
    }
    part
}

fn and(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (a, b) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::new();
    let mut j = 0;
    for &x in a {
        j += b[j..].partition_point(|&y| y < x);
        if j < b.len() && b[j] == x {
            out.push(x);
        }
    }
    out
}

fn or(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (x, y) = (a[i], b[j]);
        out.push(x.min(y));
        i += (x <= y) as usize;
        j += (y <= x) as usize;
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

fn not(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut j = 0;
    for &x in a {
        j += b[j..].partition_point(|&y| y < x);
        if j == b.len() || b[j] != x {
            out.push(x);
        }
    }
    out
}

// A running BOOL result: a sorted list while it is small, a bitmap over all documents once large.
enum Set {
    List(Vec<u32>),
    Bits(Vec<u64>),
}

fn has(b: &[u64], x: u32) -> bool {
    b[x as usize / 64] >> (x % 64) & 1 == 1
}

fn set_bits(b: &mut [u64], list: &[u32]) {
    for &x in list {
        b[x as usize / 64] |= 1 << (x % 64);
    }
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = args.get(1).map_or("/data", |s| s.as_str());
    let out_dir = args.get(2).map_or("/out", |s| s.as_str());
    let nt = thread::available_parallelism().map_or(1, |n| n.get());

    // 1. Queries first: number their terms and build the phrase automaton.
    let qtext = std::fs::read(format!("{data_dir}/queries.txt")).unwrap();
    let mut terms = Terms { keys: Vec::new(), slots: vec![0; 1 << 20] };
    let mut queries = Vec::new();
    let mut phrases = Vec::new();
    for line in qtext.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix(b"PHRASE \"") {
            phrases.push(rest[..rest.len() - 1].split(|&b| b == b' ').map(|w| terms.id(w)).collect::<Vec<u32>>());
            queries.push(Query::Phrase(phrases.len() as u32 - 1));
        } else {
            let words: Vec<&[u8]> = line[5..].split(|&b| b == b' ').collect();
            let mut q = vec![(0, terms.id(words[0]))];
            for pair in words[1..].chunks(2) {
                let op = match pair[0] {
                    b"AND" => 1,
                    b"OR" => 2,
                    _ => 3,
                };
                q.push((op, terms.id(pair[1])));
            }
            queries.push(Query::Bool(q));
        }
    }
    let nterms = terms.keys.len();
    let (mut in_bool, mut in_phrase) = (vec![false; nterms], vec![false; nterms]);
    for q in &queries {
        if let Query::Bool(q) = q {
            for &(_, t) in q {
                in_bool[t as usize] = true;
            }
        }
    }
    for p in &phrases {
        for &t in p {
            in_phrase[t as usize] = true;
        }
    }
    let (ac, ends) = build(&phrases);
    let nodes = ac.fail.len();

    // 2. Scan the documents in parallel, one range of whole lines per thread.
    let d = load(&format!("{data_dir}/docs.txt"));
    let mut bounds = vec![0];
    for i in 1..nt {
        let mut x = (d.len() * i / nt).max(bounds[i - 1]);
        while x > 0 && x < d.len() && d[x - 1] != b'\n' {
            x += 1;
        }
        bounds.push(x);
    }
    bounds.push(d.len());
    let parts: Vec<Part> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (part, terms, in_bool, in_phrase, ac) = (&d[bounds[i]..bounds[i + 1]], &terms, &in_bool, &in_phrase, &ac);
            sc.spawn(move || scan(part, terms, in_bool, in_phrase, ac, nodes))
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 3. Posting lists of the bool terms in one array; threads hold ascending doc ranges, so
    //    placing them term by term, thread by thread, keeps every list sorted.
    let ndocs = parts.iter().map(|p| p.docs).max().unwrap_or(0);
    let mut start = vec![0usize; nterms + 1];
    let mut at: Vec<Vec<usize>> = vec![vec![0; nterms]; nt];
    let mut total = 0;
    for t in 0..nterms {
        start[t] = total;
        for (i, part) in parts.iter().enumerate() {
            at[i][t] = total;
            total += part.tally[t] as usize;
        }
    }
    start[nterms] = total;
    let mut postings = vec![0u32; total];
    let out = Shared(postings.as_mut_ptr());
    thread::scope(|sc| {
        for (part, at) in parts.iter().zip(&mut at) {
            sc.spawn(move || {
                for &(t, doc) in &part.pairs {
                    unsafe { *out.at(at[t as usize]) = doc };
                    at[t as usize] += 1;
                }
            });
        }
    });
    drop(at);

    // Frequent terms also get a bitmap over all documents, so AND, OR and NOT against them cost
    // a word-wise pass or one bit test per element of the other side.
    let words = ndocs.div_ceil(64);
    let big_min = words * 2;
    let big: Vec<u32> = (0..nterms as u32).filter(|&t| start[t as usize + 1] - start[t as usize] > big_min).collect();
    let mut bitmap_of = vec![NONE; nterms];
    for (i, &t) in big.iter().enumerate() {
        bitmap_of[t as usize] = i as u32;
    }
    let bitmaps: Vec<Vec<u64>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (big, start, postings) = (&big, &start, &postings);
            sc.spawn(move || big[big.len() * i / nt..big.len() * (i + 1) / nt].iter().map(|&t| {
                let mut b = vec![0u64; words];
                set_bits(&mut b, &postings[start[t as usize]..start[t as usize + 1]]);
                b
            }).collect::<Vec<_>>())
        }).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });

    let mut hits: Vec<(u32, u32)> = parts.into_iter().flat_map(|p| p.hits).collect();
    hits.sort_unstable();

    // 4. Answer the queries in parallel.
    let next = AtomicUsize::new(0);
    let lines: Vec<Vec<(usize, Vec<u8>)>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|_| {
            let (queries, next, start, postings, hits, ends, bitmap_of, bitmaps) = (&queries, &next, &start, &postings, &hits, &ends, &bitmap_of, &bitmaps);
            sc.spawn(move || {
                let list = |t: u32| &postings[start[t as usize]..start[t as usize + 1]];
                let mut done = Vec::new();
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(q) = queries.get(i) else { return done };
                    let bits = |t: u32| (bitmap_of[t as usize] != NONE).then(|| &bitmaps[bitmap_of[t as usize] as usize][..]);
                    let result = match q {
                        Query::Bool(q) => {
                            let t = q[0].1;
                            let mut r = match bits(t) {
                                Some(b) => Set::Bits(b.to_vec()),
                                None => Set::List(list(t).to_vec()),
                            };
                            for &(op, t) in &q[1..] {
                                let (l, b) = (list(t), bits(t));
                                r = match (op, r) {
                                    (1, Set::List(mut v)) => {
                                        match b {
                                            Some(b) => v.retain(|&x| has(b, x)),
                                            None => v = and(&v, l),
                                        }
                                        Set::List(v)
                                    }
                                    (1, Set::Bits(mut v)) => match b {
                                        Some(b) => {
                                            v.iter_mut().zip(b).for_each(|(x, y)| *x &= y);
                                            Set::Bits(v)
                                        }
                                        None => Set::List(l.iter().copied().filter(|&x| has(&v, x)).collect()),
                                    },
                                    (2, Set::List(v)) if b.is_none() && v.len() + l.len() <= big_min => Set::List(or(&v, l)),
                                    (2, r) => {
                                        let mut v = match r {
                                            Set::Bits(v) => v,
                                            Set::List(v) => {
                                                let mut bv = vec![0u64; words];
                                                set_bits(&mut bv, &v);
                                                bv
                                            }
                                        };
                                        match b {
                                            Some(b) => v.iter_mut().zip(b).for_each(|(x, y)| *x |= y),
                                            None => set_bits(&mut v, l),
                                        }
                                        Set::Bits(v)
                                    }
                                    (_, Set::List(mut v)) => {
                                        match b {
                                            Some(b) => v.retain(|&x| !has(b, x)),
                                            None => v = not(&v, l),
                                        }
                                        Set::List(v)
                                    }
                                    (_, Set::Bits(mut v)) => {
                                        match b {
                                            Some(b) => v.iter_mut().zip(b).for_each(|(x, y)| *x &= !y),
                                            None => l.iter().for_each(|&x| v[x as usize / 64] &= !(1 << (x % 64))),
                                        }
                                        Set::Bits(v)
                                    }
                                };
                            }
                            r
                        }
                        Query::Phrase(p) => {
                            let node = ends[*p as usize];
                            let lo = hits.partition_point(|&(x, _)| x < node);
                            let hi = hits.partition_point(|&(x, _)| x <= node);
                            Set::List(hits[lo..hi].iter().map(|&(_, doc)| doc).collect())
                        }
                    };
                    // count, sum of ids, and the ten smallest ids
                    let (mut count, mut sum, mut first) = (0u64, 0u64, Vec::new());
                    let mut take = |x: u32| {
                        count += 1;
                        sum += x as u64;
                        if first.len() < 10 {
                            first.push(x);
                        }
                    };
                    match &result {
                        Set::List(v) => v.iter().for_each(|&x| take(x)),
                        Set::Bits(v) => {
                            for (w, &word) in v.iter().enumerate() {
                                let mut b = word;
                                while b != 0 {
                                    take((w * 64) as u32 + b.trailing_zeros());
                                    b &= b - 1;
                                }
                            }
                        }
                    }
                    let mut o = Vec::new();
                    write!(o, "{count} {sum}").unwrap();
                    for x in &first {
                        write!(o, " {x}").unwrap();
                    }
                    o.push(b'\n');
                    done.push((i, o));
                }
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut out = vec![Vec::new(); queries.len()];
    for (i, o) in lines.into_iter().flatten() {
        out[i] = o;
    }
    std::fs::write(format!("{out_dir}/results.txt"), out.concat()).unwrap();
}
