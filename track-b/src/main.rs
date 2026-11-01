// Speed Demon 2026, Track B (Index). See README.md for the approach.
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::thread;

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

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

// Compact prefix filter (65,536 bits = 8 KB bitmap for 2-byte prefixes, 256 bits = 32 B for 1-byte terms).
struct PrefixFilter {
    table: [u64; 1024],
    len1: [u64; 4],
}

impl PrefixFilter {
    fn new() -> Self {
        Self {
            table: [0u64; 1024],
            len1: [0u64; 4],
        }
    }

    #[inline]
    fn insert(&mut self, word: &[u8]) {
        if word.len() == 1 {
            let b = word[0] as usize;
            self.len1[b / 64] |= 1u64 << (b % 64);
        } else if word.len() >= 2 {
            let p = ((word[0] as usize) << 8) | (word[1] as usize);
            self.table[p / 64] |= 1u64 << (p % 64);
        }
    }

    #[inline(always)]
    fn may_contain(&self, key: &[u8; 32], len: usize) -> bool {
        if len == 1 {
            let b = key[0] as usize;
            (self.len1[b / 64] & (1u64 << (b % 64))) != 0
        } else if len >= 2 {
            let p = ((key[0] as usize) << 8) | (key[1] as usize);
            (self.table[p / 64] & (1u64 << (p % 64))) != 0
        } else {
            false
        }
    }
}

// The distinct query terms, numbered in order of first appearance.
struct Terms {
    keys: Vec<Key>,
    slots: Vec<u32>, // id + 1, 0 = empty
    filter: PrefixFilter,
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
        self.filter.insert(word);
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

// Aho-Corasick over term ids with O(1) direct root transitions and sorted flat non-root edges.
struct Automaton {
    root_goto: Vec<u32>,    // direct O(1) transitions from root state 0
    edge_start: Vec<u32>,   // slice offset into edges for non-root states
    edges: Vec<(u32, u32)>, // sorted by term (t, v) for non-root states
    fail: Vec<u32>,
    out: Vec<u32>,
}

impl Automaton {
    #[inline(always)]
    fn step(&self, mut s: u32, t: u32) -> u32 {
        if s == 0 {
            return self.root_goto[t as usize];
        }
        loop {
            let lo = self.edge_start[s as usize] as usize;
            let hi = self.edge_start[s as usize + 1] as usize;
            let len = hi - lo;
            if len == 1 {
                let (term, next) = self.edges[lo];
                if term == t {
                    return next;
                }
            } else if len > 1 {
                let slice = &self.edges[lo..hi];
                if let Ok(idx) = slice.binary_search_by_key(&t, |&(term, _)| term) {
                    return slice[idx].1;
                }
            }
            s = self.fail[s as usize];
            if s == 0 {
                return self.root_goto[t as usize];
            }
        }
    }
}

fn build(phrases: &[Vec<u32>], nterms: usize) -> (Automaton, Vec<u32>) {
    let mut children: Vec<Vec<(u32, u32)>> = vec![vec![]];
    let mut root_goto = vec![0u32; nterms];
    let mut ends = Vec::with_capacity(phrases.len());

    for p in phrases {
        let mut s = 0u32;
        for &t in p {
            let mut v = if s == 0 {
                root_goto[t as usize]
            } else {
                children[s as usize]
                    .iter()
                    .find(|&&(term, _)| term == t)
                    .map(|&(_, node)| node)
                    .unwrap_or(0)
            };
            if v == 0 {
                v = children.len() as u32;
                children.push(Vec::new());
                if s == 0 {
                    root_goto[t as usize] = v;
                }
                children[s as usize].push((t, v));
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

    let mut edge_start = Vec::with_capacity(n + 1);
    let mut edges = Vec::new();
    for s in 0..n {
        edge_start.push(edges.len() as u32);
        if s > 0 {
            children[s].sort_unstable_by_key(|&(term, _)| term);
            edges.extend_from_slice(&children[s]);
        }
    }
    edge_start.push(edges.len() as u32);

    let mut a = Automaton {
        root_goto,
        edge_start,
        edges,
        fail: vec![0; n],
        out: vec![NONE; n],
    };

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
    lists: Vec<Vec<u32>>,  // term -> docs in this thread
    hits: Vec<(u32, u32)>, // (phrase end node, doc), each pair once
    docs: usize,           // last doc id seen + 1
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn classify_32(ptr: *const u8) -> (u32, [u8; 32]) {
    let v = _mm256_loadu_si256(ptr as *const __m256i);
    let is_digit = _mm256_and_si256(
        _mm256_cmpgt_epi8(v, _mm256_set1_epi8(47)),
        _mm256_cmpgt_epi8(_mm256_set1_epi8(58), v),
    );
    let is_upper = _mm256_and_si256(
        _mm256_cmpgt_epi8(v, _mm256_set1_epi8(64)),
        _mm256_cmpgt_epi8(_mm256_set1_epi8(91), v),
    );
    let is_lower = _mm256_and_si256(
        _mm256_cmpgt_epi8(v, _mm256_set1_epi8(96)),
        _mm256_cmpgt_epi8(_mm256_set1_epi8(123), v),
    );
    let is_token = _mm256_or_si256(is_digit, _mm256_or_si256(is_upper, is_lower));
    let mask = _mm256_movemask_epi8(is_token) as u32;
    let lower_v = _mm256_add_epi8(v, _mm256_and_si256(is_upper, _mm256_set1_epi8(32)));
    let mut out = [0u8; 32];
    _mm256_storeu_si256(out.as_mut_ptr() as *mut __m256i, lower_v);
    (mask, out)
}

#[inline(always)]
fn on_token_action(
    key: &[u8; 32],
    len: usize,
    doc: u32,
    terms: &Terms,
    in_bool: &[bool],
    in_phrase: &[bool],
    ac: &Automaton,
    state: &mut u32,
    part: &mut Part,
    last: &mut [u32],
    last_hit: &mut [u32],
) {
    if len > 32 || !terms.filter.may_contain(key, len) {
        *state = 0;
        return;
    }
    let (_, t) = terms.find(key);
    if t == NONE {
        *state = 0;
        return;
    }
    if in_bool[t as usize] && last[t as usize] != doc {
        last[t as usize] = doc;
        part.lists[t as usize].push(doc);
    }
    if !in_phrase[t as usize] {
        *state = 0;
    } else {
        *state = ac.step(*state, t);
        if *state != 0 {
            let mut x = ac.out[*state as usize];
            while x != NONE {
                if last_hit[x as usize] != doc {
                    last_hit[x as usize] = doc;
                    part.hits.push((x, doc));
                }
                x = ac.out[ac.fail[x as usize] as usize];
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_doc_avx2(
    text: &[u8],
    doc: u32,
    terms: &Terms,
    in_bool: &[bool],
    in_phrase: &[bool],
    ac: &Automaton,
    part: &mut Part,
    last: &mut [u32],
    last_hit: &mut [u32],
) {
    let mut state = 0u32;
    let mut cur = [0u8; 32];
    let mut cur_len = 0usize;

    let mut p = 0;
    while p + 32 <= text.len() {
        let (mask, out) = classify_32(text.as_ptr().add(p));
        let mut off = 0;
        while off < 32 {
            let bit = (mask >> off) & 1;
            if bit != 0 {
                let sub = mask >> off;
                let len = (!sub).trailing_zeros().min(32 - off as u32) as usize;
                let take = len.min(32 - cur_len.min(32));
                cur[cur_len.min(32)..cur_len.min(32) + take].copy_from_slice(&out[off..off + take]);
                cur_len += len;
                off += len;
            } else {
                if cur_len > 0 {
                    on_token_action(&cur, cur_len, doc, terms, in_bool, in_phrase, ac, &mut state, part, last, last_hit);
                    cur = [0; 32];
                    cur_len = 0;
                }
                let sub = mask >> off;
                let sep = sub.trailing_zeros().min(32 - off as u32) as usize;
                off += sep.max(1);
            }
        }
        p += 32;
    }

    while p < text.len() {
        let b = text[p];
        p += 1;
        let c = match b {
            b'a'..=b'z' | b'0'..=b'9' => b,
            b'A'..=b'Z' => b + 32,
            _ => 0,
        };
        if c != 0 {
            if cur_len < 32 {
                cur[cur_len] = c;
            }
            cur_len += 1;
        } else if cur_len > 0 {
            on_token_action(&cur, cur_len, doc, terms, in_bool, in_phrase, ac, &mut state, part, last, last_hit);
            cur = [0; 32];
            cur_len = 0;
        }
    }

    if cur_len > 0 {
        on_token_action(&cur, cur_len, doc, terms, in_bool, in_phrase, ac, &mut state, part, last, last_hit);
    }
}

fn scan_doc_scalar(
    text: &[u8],
    doc: u32,
    terms: &Terms,
    in_bool: &[bool],
    in_phrase: &[bool],
    ac: &Automaton,
    part: &mut Part,
    last: &mut [u32],
    last_hit: &mut [u32],
) {
    let mut state = 0u32;
    let mut key = [0u8; 32];
    let mut len = 0usize;
    for &b in text {
        let c = match b {
            b'a'..=b'z' | b'0'..=b'9' => b,
            b'A'..=b'Z' => b + 32,
            _ => 0,
        };
        if c != 0 {
            if len < 32 {
                key[len] = c;
            }
            len += 1;
        } else if len > 0 {
            on_token_action(&key, len, doc, terms, in_bool, in_phrase, ac, &mut state, part, last, last_hit);
            key = [0; 32];
            len = 0;
        }
    }
    if len > 0 {
        on_token_action(&key, len, doc, terms, in_bool, in_phrase, ac, &mut state, part, last, last_hit);
    }
}

fn scan(d: &[u8], terms: &Terms, in_bool: &[bool], in_phrase: &[bool], ac: &Automaton, nodes: usize, use_avx2: bool) -> Part {
    let nterms = terms.keys.len();
    let mut part = Part { lists: vec![Vec::new(); nterms], hits: Vec::new(), docs: 0 };
    let mut last = vec![NONE; nterms];
    let mut last_hit = vec![NONE; nodes];
    let mut p = 0;
    while p < d.len() {
        let mut doc = 0u32;
        while p < d.len() && d[p] != b'\t' {
            doc = doc * 10 + (d[p] - b'0') as u32;
            p += 1;
        }
        if p >= d.len() {
            break;
        }
        p += 1;
        part.docs = doc as usize + 1;
        let nl = match d[p..].iter().position(|&b| b == b'\n') {
            Some(i) => p + i,
            None => d.len(),
        };
        let text = &d[p..nl];
        p = nl + 1;

        #[cfg(target_arch = "x86_64")]
        if use_avx2 {
            unsafe {
                scan_doc_avx2(text, doc, terms, in_bool, in_phrase, ac, &mut part, &mut last, &mut last_hit);
            }
            continue;
        }

        scan_doc_scalar(text, doc, terms, in_bool, in_phrase, ac, &mut part, &mut last, &mut last_hit);
    }
    part
}

fn and(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (a, b) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(a.len());
    if a.len() * 16 < b.len() {
        let mut j = 0;
        for &x in a {
            j += b[j..].partition_point(|&y| y < x);
            if j < b.len() && b[j] == x {
                out.push(x);
            }
        }
    } else {
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            let (x, y) = (a[i], b[j]);
            if x == y {
                out.push(x);
                i += 1;
                j += 1;
            } else if x < y {
                i += 1;
            } else {
                j += 1;
            }
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
    let mut out = Vec::with_capacity(a.len());
    if a.len() * 16 < b.len() {
        let mut j = 0;
        for &x in a {
            j += b[j..].partition_point(|&y| y < x);
            if j == b.len() || b[j] != x {
                out.push(x);
            }
        }
    } else {
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            let (x, y) = (a[i], b[j]);
            if x < y {
                out.push(x);
                i += 1;
            } else if x == y {
                i += 1;
                j += 1;
            } else {
                j += 1;
            }
        }
        out.extend_from_slice(&a[i..]);
    }
    out
}

#[inline(always)]
fn bitmap_and(dst: &mut [u64], src: &[u64]) {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        unsafe {
            let mut i = 0;
            while i + 4 <= dst.len() {
                let d = _mm256_loadu_si256(dst.as_ptr().add(i) as *const __m256i);
                let s = _mm256_loadu_si256(src.as_ptr().add(i) as *const __m256i);
                let r = _mm256_and_si256(d, s);
                _mm256_storeu_si256(dst.as_mut_ptr().add(i) as *mut __m256i, r);
                i += 4;
            }
            while i < dst.len() {
                dst[i] &= src[i];
                i += 1;
            }
            return;
        }
    }
    dst.iter_mut().zip(src).for_each(|(x, y)| *x &= y);
}

#[inline(always)]
fn bitmap_or(dst: &mut [u64], src: &[u64]) {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        unsafe {
            let mut i = 0;
            while i + 4 <= dst.len() {
                let d = _mm256_loadu_si256(dst.as_ptr().add(i) as *const __m256i);
                let s = _mm256_loadu_si256(src.as_ptr().add(i) as *const __m256i);
                let r = _mm256_or_si256(d, s);
                _mm256_storeu_si256(dst.as_mut_ptr().add(i) as *mut __m256i, r);
                i += 4;
            }
            while i < dst.len() {
                dst[i] |= src[i];
                i += 1;
            }
            return;
        }
    }
    dst.iter_mut().zip(src).for_each(|(x, y)| *x |= y);
}

#[inline(always)]
fn bitmap_not(dst: &mut [u64], src: &[u64]) {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        unsafe {
            let mut i = 0;
            while i + 4 <= dst.len() {
                let d = _mm256_loadu_si256(dst.as_ptr().add(i) as *const __m256i);
                let s = _mm256_loadu_si256(src.as_ptr().add(i) as *const __m256i);
                let r = _mm256_andnot_si256(s, d);
                _mm256_storeu_si256(dst.as_mut_ptr().add(i) as *mut __m256i, r);
                i += 4;
            }
            while i < dst.len() {
                dst[i] &= !src[i];
                i += 1;
            }
            return;
        }
    }
    dst.iter_mut().zip(src).for_each(|(x, y)| *x &= !y);
}

// A running BOOL result: a sorted list while it is small, a bitmap over all documents once large, or a phrase slice.
enum Set<'a> {
    List(Vec<u32>),
    Bits(Vec<u64>),
    PhraseSlice(&'a [(u32, u32)]),
}

#[inline(always)]
fn has(b: &[u64], x: u32) -> bool {
    (b[x as usize / 64] >> (x % 64)) & 1 == 1
}

#[inline(always)]
fn set_bits(b: &mut [u64], list: &[u32]) {
    for &x in list {
        b[x as usize / 64] |= 1 << (x % 64);
    }
}


fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = args.get(1).map_or("/data", |s| s.as_str());
    let out_dir = args.get(2).map_or("/out", |s| s.as_str());
    let nt = thread::available_parallelism().map_or(1, |n| n.get());

    // 1. Queries first: number their terms and build the phrase automaton.
    let qtext = std::fs::read(format!("{data_dir}/queries.txt")).unwrap();
    let mut terms = Terms { keys: Vec::new(), slots: vec![0; 1 << 20], filter: PrefixFilter::new() };
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
    let (ac, ends) = build(&phrases, nterms);
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
    let use_avx2 = is_x86_feature_detected!("avx2");
    let parts: Vec<Part> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (part, terms, in_bool, in_phrase, ac) = (&d[bounds[i]..bounds[i + 1]], &terms, &in_bool, &in_phrase, &ac);
            sc.spawn(move || scan(part, terms, in_bool, in_phrase, ac, nodes, use_avx2))
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 3. Assemble posting lists and dense bitmaps without intermediate global pairs.
    // Frequent terms (> big_min) are stored exclusively as dense bitmaps;
    // sparse terms are stored as compact sorted slices in postings.
    let ndocs = parts.iter().map(|p| p.docs).max().unwrap_or(0);
    let words = ndocs.div_ceil(64);
    let big_min = words * 2;

    let mut tally = vec![0usize; nterms];
    for p in &parts {
        for t in 0..nterms {
            tally[t] += p.lists[t].len();
        }
    }

    let mut bitmap_of = vec![NONE; nterms];
    let mut big = Vec::new();
    let mut start = vec![0usize; nterms + 1];
    let mut sparse_total = 0;

    for t in 0..nterms {
        if tally[t] > big_min {
            bitmap_of[t] = big.len() as u32;
            big.push(t as u32);
            start[t] = sparse_total;
        } else {
            start[t] = sparse_total;
            sparse_total += tally[t];
        }
    }
    start[nterms] = sparse_total;

    let mut postings = vec![0u32; sparse_total];
    let mut at = start.clone();
    for p in &parts {
        for t in 0..nterms {
            if bitmap_of[t] == NONE {
                let l = &p.lists[t];
                let off = at[t];
                postings[off..off + l.len()].copy_from_slice(l);
                at[t] += l.len();
            }
        }
    }

    let mut bitmaps = Vec::with_capacity(big.len());
    for &t in &big {
        let mut b = vec![0u64; words];
        for p in &parts {
            set_bits(&mut b, &p.lists[t as usize]);
        }
        bitmaps.push(b);
    }

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
                                            bitmap_and(&mut v, b);
                                            Set::Bits(v)
                                        }
                                        None => {
                                            let mut out = Vec::with_capacity(l.len());
                                            for &x in l {
                                                if has(&v, x) {
                                                    out.push(x);
                                                }
                                            }
                                            Set::List(out)
                                        }
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
                                            Set::PhraseSlice(_) => unreachable!(),
                                        };
                                        match b {
                                            Some(b) => bitmap_or(&mut v, b),
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
                                            Some(b) => bitmap_not(&mut v, b),
                                            None => l.iter().for_each(|&x| v[x as usize / 64] &= !(1u64 << (x % 64))),
                                        }
                                        Set::Bits(v)
                                    }
                                    (_, Set::PhraseSlice(_)) => unreachable!(),
                                };
                            }
                            r
                        }
                        Query::Phrase(p) => {
                            let node = ends[*p as usize];
                            let lo = hits.partition_point(|&(x, _)| x < node);
                            let hi = hits.partition_point(|&(x, _)| x <= node);
                            Set::PhraseSlice(&hits[lo..hi])
                        }
                    };
                    // count, sum of ids, and the ten smallest ids
                    let (count, sum);
                    let mut first = Vec::with_capacity(10);
                    match &result {
                        Set::List(v) => {
                            count = v.len() as u64;
                            sum = v.iter().map(|&x| x as u64).sum();
                            first.extend(v.iter().take(10).copied());
                        }
                        Set::PhraseSlice(slice) => {
                            count = slice.len() as u64;
                            sum = slice.iter().map(|&(_, doc)| doc as u64).sum();
                            first.extend(slice.iter().take(10).map(|&(_, doc)| doc));
                        }
                        Set::Bits(v) => {
                            count = v.iter().map(|&w| w.count_ones() as u64).sum();
                            for (w, &word) in v.iter().enumerate() {
                                let mut b = word;
                                while b != 0 {
                                    first.push((w * 64) as u32 + b.trailing_zeros());
                                    if first.len() == 10 {
                                        break;
                                    }
                                    b &= b - 1;
                                }
                                if first.len() == 10 {
                                    break;
                                }
                            }
                            let mut total_sum = 0u64;
                            for (w, &word) in v.iter().enumerate() {
                                if word == 0 {
                                    continue;
                                }
                                let base = (w * 64) as u64;
                                if word == u64::MAX {
                                    total_sum += base * 64 + 2016;
                                } else {
                                    let pop = word.count_ones() as u64;
                                    total_sum += base * pop;
                                    let mut b = word;
                                    while b != 0 {
                                        total_sum += b.trailing_zeros() as u64;
                                        b &= b - 1;
                                    }
                                }
                            }
                            sum = total_sum;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_filter_basic() {
        let mut filter = PrefixFilter::new();
        filter.insert(b"apple");
        filter.insert(b"banana");
        filter.insert(b"c");
        filter.insert(b"123");

        let make_key = |s: &[u8]| {
            let mut k = [0u8; 32];
            k[..s.len()].copy_from_slice(s);
            k
        };

        // All inserted terms must pass
        assert!(filter.may_contain(&make_key(b"apple"), 5));
        assert!(filter.may_contain(&make_key(b"banana"), 6));
        assert!(filter.may_contain(&make_key(b"c"), 1));
        assert!(filter.may_contain(&make_key(b"123"), 3));

        // Other terms sharing the 2-byte prefix pass filter (admissible filter)
        assert!(filter.may_contain(&make_key(b"apricot"), 7));
        assert!(filter.may_contain(&make_key(b"band"), 4));

        // Terms with uninserted prefixes must be rejected
        assert!(!filter.may_contain(&make_key(b"zebra"), 5));
        assert!(!filter.may_contain(&make_key(b"orange"), 6));
        assert!(!filter.may_contain(&make_key(b"d"), 1));
        assert!(!filter.may_contain(&make_key(b"999"), 3));

        // Empty and len 0
        assert!(!filter.may_contain(&make_key(b""), 0));
        assert!(!filter.may_contain(&make_key(b"a"), 0));
    }

    #[test]
    fn test_prefix_filter_all_query_terms_pass() {
        let mut terms = Terms {
            keys: Vec::new(),
            slots: vec![0; 1024],
            filter: PrefixFilter::new(),
        };

        let sample_words: &[&[u8]] = &[
            b"a", b"1", b"in", b"the", b"query", b"search", b"demon",
            b"optimization", b"aho", b"corasick", b"bitmap", b"vector",
        ];

        for &w in sample_words {
            terms.id(w);
        }

        for &w in sample_words {
            let mut k = [0u8; 32];
            k[..w.len()].copy_from_slice(w);
            assert!(
                terms.filter.may_contain(&k, w.len()),
                "Query term {:?} must pass prefix filter",
                std::str::from_utf8(w).unwrap()
            );
            let (_, id) = terms.find(&k);
            assert_ne!(id, NONE, "Query term must be found in terms");
        }

        // Check non-existent term rejection
        let mut k = [0u8; 32];
        k[..5].copy_from_slice(b"zztop");
        assert!(!terms.filter.may_contain(&k, 5));
    }

    #[test]
    fn test_ac_phrase_transitions_and_failure_links() {
        // Phrases: "bail bond" (terms [0, 1]), "bail bail bond" (terms [0, 0, 1]), "bond bail" (terms [1, 0])
        let phrases = vec![
            vec![0, 1],       // Phrase 0: bail bond
            vec![0, 0, 1],    // Phrase 1: bail bail bond
            vec![1, 0],       // Phrase 2: bond bail
        ];
        let nterms = 3;
        let (ac, ends) = build(&phrases, nterms);

        // From root: 0 -> bail (term 0) -> node 1
        let s1 = ac.step(0, 0);
        assert_eq!(s1, 1);
        // From root: 0 -> bond (term 1) -> node 5 (or other allocated node)
        let s_bond = ac.step(0, 1);
        assert_ne!(s_bond, 0);
        // Non-initial term (term 2) from root: returns 0
        assert_eq!(ac.step(0, 2), 0);

        // Match sequence: bail bail bond
        // 1. token 0 (bail)
        let mut st = ac.step(0, 0);
        assert_eq!(ac.out[st as usize], NONE);
        // 2. token 0 (bail) -> enters node for "bail bail"
        st = ac.step(st, 0);
        assert_eq!(ac.out[st as usize], NONE);
        // 3. token 1 (bond) -> completes "bail bail bond" (ends[1]) and suffix "bail bond" (ends[0])
        st = ac.step(st, 1);
        assert_eq!(st, ends[1]);
        // out link must match phrase 1 itself
        assert_eq!(ac.out[st as usize], ends[1]);
        // fail chain must reach phrase 0
        let next_out = ac.out[ac.fail[st as usize] as usize];
        assert_eq!(next_out, ends[0]);
    }

    #[test]
    fn test_ac_eight_token_phrase() {
        // An 8-token phrase: 0 1 2 3 4 5 6 7
        let p: Vec<u32> = (0..8).collect();
        let phrases = vec![p.clone()];
        let (ac, ends) = build(&phrases, 10);

        let mut st = 0;
        for &t in &p {
            st = ac.step(st, t);
        }
        assert_eq!(st, ends[0]);
        assert_eq!(ac.out[st as usize], ends[0]);
    }

    #[test]
    fn test_ac_overlapping_phrases() {
        // Overlapping: "a b c" [0, 1, 2] and "b c d" [1, 2, 3]
        let phrases = vec![vec![0, 1, 2], vec![1, 2, 3]];
        let (ac, ends) = build(&phrases, 4);

        // Sequence: 0, 1, 2, 3
        let s0 = ac.step(0, 0);
        let s1 = ac.step(s0, 1);
        let s2 = ac.step(s1, 2);
        assert_eq!(s2, ends[0]); // matches "a b c"

        let s3 = ac.step(s2, 3);
        assert_eq!(s3, ends[1]); // matches "b c d" via fail transition!
    }

    #[test]
    fn test_boolean_engine_set_operations() {
        let a = vec![1, 5, 10, 20, 25, 30];
        let b = vec![5, 15, 20, 35];
        assert_eq!(and(&a, &b), vec![5, 20]);
        assert_eq!(or(&a, &b), vec![1, 5, 10, 15, 20, 25, 30, 35]);
        assert_eq!(not(&a, &b), vec![1, 10, 25, 30]);

        // Left to right: (a OR b) NOT a
        let a_or_b = or(&a, &b);
        let res = not(&a_or_b, &a);
        assert_eq!(res, vec![15, 35]);
    }

    #[test]
    fn test_bitmap_operations_and_popcount() {
        let mut b1 = vec![0u64; 8];
        let mut b2 = vec![0u64; 8];

        set_bits(&mut b1, &[0, 63, 64, 127, 200]);
        set_bits(&mut b2, &[63, 127, 300]);

        let mut b_and = b1.clone();
        bitmap_and(&mut b_and, &b2);
        assert_eq!(b_and.iter().map(|&w| w.count_ones()).sum::<u32>(), 2);
        assert!(has(&b_and, 63));
        assert!(has(&b_and, 127));
        assert!(!has(&b_and, 0));

        let mut b_or = b1.clone();
        bitmap_or(&mut b_or, &b2);
        assert_eq!(b_or.iter().map(|&w| w.count_ones()).sum::<u32>(), 6);

        let mut b_not = b1.clone();
        bitmap_not(&mut b_not, &b2);
        assert_eq!(b_not.iter().map(|&w| w.count_ones()).sum::<u32>(), 3);
        assert!(has(&b_not, 0));
        assert!(has(&b_not, 64));
        assert!(has(&b_not, 200));
        assert!(!has(&b_not, 63));
    }

    #[test]
    fn test_count_sum_first_exactness() {
        // Test list with >10 items: 10..25 (15 items)
        let list: Vec<u32> = (10..25).collect();
        let expected_count = 15u64;
        let expected_sum: u64 = (10..25).map(|x| x as u64).sum();
        let expected_first: Vec<u32> = (10..20).collect();

        // Evaluate via bitmap
        let mut b = vec![0u64; 4];
        set_bits(&mut b, &list);

        let b_count: u64 = b.iter().map(|&w| w.count_ones() as u64).sum();
        assert_eq!(b_count, expected_count);

        let mut b_first = Vec::new();
        for (w, &word) in b.iter().enumerate() {
            let mut val = word;
            while val != 0 {
                b_first.push((w * 64) as u32 + val.trailing_zeros());
                if b_first.len() == 10 {
                    break;
                }
                val &= val - 1;
            }
            if b_first.len() == 10 {
                break;
            }
        }
        assert_eq!(b_first, expected_first);

        let mut b_sum = 0u64;
        for (w, &word) in b.iter().enumerate() {
            if word == 0 {
                continue;
            }
            let base = (w * 64) as u64;
            if word == u64::MAX {
                b_sum += base * 64 + 2016;
            } else {
                let pop = word.count_ones() as u64;
                b_sum += base * pop;
                let mut val = word;
                while val != 0 {
                    b_sum += val.trailing_zeros() as u64;
                    val &= val - 1;
                }
            }
        }
        assert_eq!(b_sum, expected_sum);
    }
}
