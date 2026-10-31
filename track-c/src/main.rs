// Speed Demon 2026, Track C (Join). See README.md for the approach.
use std::io::Write;
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

// A text field at *p: its raw bytes (between the quotes, if quoted) and how many "" escapes they
// hold, with *p moved past the comma after it. Quoting is canonical for non-empty values, so the
// raw bytes identify the value, and its length is the raw length minus the escapes.
fn text<'a>(d: &'a [u8], p: &mut usize) -> (&'a [u8], usize) {
    let s = *p;
    if d[s] != b'"' {
        let e = find(d, s, b',');
        *p = e + 1;
        return (&d[s..e], 0);
    }
    let (mut i, mut escapes) = (s + 1, 0);
    loop {
        i = find(d, i, b'"');
        if d[i + 1] != b'"' {
            break;
        }
        escapes += 1;
        i += 2;
    }
    *p = i + 2;
    (&d[s + 1..i], escapes)
}

fn hash(s: &[u8]) -> u64 {
    let mut h = s.len() as u64;
    for c in s.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        h = (h.rotate_left(5) ^ u64::from_le_bytes(w)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    h ^ h >> 29
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

// A set of distinct names, each with a bitmask of the groups it occurs in (`words` u64s per name).
struct Names<'a> {
    words: usize,
    names: Vec<&'a [u8]>,
    hashes: Vec<u64>,
    masks: Vec<u64>,
    slots: Vec<u32>, // open addressing, id + 1, 0 = empty
}

impl<'a> Names<'a> {
    fn new(words: usize) -> Self {
        Names { words, names: vec![], hashes: vec![], masks: vec![], slots: vec![0; 1 << 12] }
    }

    // The mask of a name, added empty if new.
    fn mask(&mut self, name: &'a [u8], h: u64) -> &mut [u64] {
        let mask = self.slots.len() - 1;
        let mut i = h as usize & mask;
        let id = loop {
            match self.slots[i] {
                0 => {
                    self.names.push(name);
                    self.hashes.push(h);
                    self.masks.resize(self.names.len() * self.words, 0);
                    self.slots[i] = self.names.len() as u32;
                    if self.names.len() * 2 > self.slots.len() {
                        self.grow();
                    }
                    break self.names.len() - 1;
                }
                s if self.hashes[s as usize - 1] == h && self.names[s as usize - 1] == name => break s as usize - 1,
                _ => i = (i + 1) & mask,
            }
        };
        &mut self.masks[id * self.words..(id + 1) * self.words]
    }

    fn grow(&mut self) {
        self.slots = vec![0; self.slots.len() * 2];
        let mask = self.slots.len() - 1;
        for (id, &h) in self.hashes.iter().enumerate() {
            let mut i = h as usize & mask;
            while self.slots[i] != 0 {
                i = (i + 1) & mask;
            }
            self.slots[i] = id as u32 + 1;
        }
    }
}

// Parse the records in d[start..end] (start is a record start, outside quotes) and aggregate them
// by group: region_of[region_id] is the region's group, and a missing region is group `unmatched`.
fn parse<'a>(d: &'a [u8], start: usize, end: usize, region_of: &[u16], unmatched: usize) -> (Vec<Agg>, Names<'a>) {
    let empty = Agg { count: 0, id_sum: 0, amount_sum: 0, min: i64::MAX, max: i64::MIN, name_bytes: 0, note_bytes: 0 };
    let mut aggs = vec![empty; unmatched + 1];
    let mut names = Names::new(unmatched / 64 + 1);
    let mut p = start;
    while p < end {
        if d[p..].starts_with(BOM) {
            p += 3;
        }
        let id = num(d, &mut p);
        let (name, name_esc) = text(d, &mut p);
        let (note, note_esc) = text(d, &mut p);
        let region = num(d, &mut p) as usize;
        let negative = d[p] == b'-';
        p += negative as usize;
        let whole = num(d, &mut p);
        let cents = (whole * 100 + (d[p] - b'0') as i64 * 10 + (d[p + 1] - b'0') as i64) * if negative { -1 } else { 1 };
        p += 2;
        p += if d.get(p) == Some(&b'\r') { 2 } else { 1 };

        let g = region_of.get(region).map_or(unmatched, |&g| g as usize);
        let a = &mut aggs[g];
        a.count += 1;
        a.id_sum += id;
        a.amount_sum += cents;
        a.min = a.min.min(cents);
        a.max = a.max.max(cents);
        a.name_bytes += (name.len() - name_esc) as i64;
        a.note_bytes += (note.len() - note_esc) as i64;
        names.mask(name, hash(name))[g / 64] |= 1 << (g % 64);
    }
    (aggs, names)
}

fn money(c: i64) -> String {
    format!("{}{}.{:02}", if c < 0 { "-" } else { "" }, c.abs() / 100, c.abs() % 100)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = args.get(1).map_or("/data", |s| s.as_str());
    let out_dir = args.get(2).map_or("/out", |s| s.as_str());
    let nt = thread::available_parallelism().map_or(1, |n| n.get());

    // 1. Regions: group = rank of the country code; a region id not listed maps to `unmatched`.
    let r = std::fs::read(format!("{data_dir}/regions.csv")).unwrap();
    let mut rows = Vec::new();
    let mut p = find(&r, 0, b'\n') + 1;
    while p < r.len() {
        let id = num(&r, &mut p) as usize;
        rows.push((id, &r[p..p + 3]));
        p += 4;
    }
    let mut codes: Vec<&[u8]> = rows.iter().map(|&(_, c)| c).collect();
    codes.sort_unstable();
    codes.dedup();
    let unmatched = codes.len();
    let mut region_of = vec![unmatched as u16; rows.iter().map(|&(id, _)| id + 1).max().unwrap_or(0)];
    for &(id, c) in &rows {
        region_of[id] = codes.binary_search(&c).unwrap() as u16;
    }

    // 2. Split records.csv at record starts. A byte is inside quotes exactly when an odd number of
    //    quotes precede it ("" escapes come in pairs), so count each chunk's quotes in parallel,
    //    then walk from each cut to the first LF outside quotes.
    let d = load(&format!("{data_dir}/records.csv"));
    let cuts: Vec<usize> = (0..=nt).map(|i| d.len() * i / nt).collect();
    let quotes: Vec<usize> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let part = &d[cuts[i]..cuts[i + 1]];
            sc.spawn(move || part.iter().filter(|&&b| b == b'"').count())
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let header = d.strip_prefix(BOM).map_or(0, |_| 3);
    let mut starts = vec![find(d, header, b'\n') + 1];
    let mut parity = 0;
    for i in 1..nt {
        parity += quotes[i - 1];
        let (mut x, mut inside) = (cuts[i], parity % 2 == 1);
        while x < d.len() && (inside || d[x] != b'\n') {
            inside ^= d[x] == b'"';
            x += 1;
        }
        starts.push((x + 1).min(d.len()).max(starts[i - 1]));
    }
    starts.push(d.len());

    // 3. Parse and aggregate every chunk in parallel.
    let parts: Vec<(Vec<Agg>, Names)> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|i| {
            let (s, e, region_of) = (starts[i], starts[i + 1], &region_of);
            sc.spawn(move || parse(d, s, e, region_of, unmatched))
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // 4. Distinct names per group: merge the threads' name sets in parallel, each thread taking the
    //    names whose hash falls in its shard, and count each group's bit over the merged masks.
    let groups = unmatched + 1;
    let distinct: Vec<Vec<i64>> = thread::scope(|sc| {
        let hs: Vec<_> = (0..nt).map(|shard| {
            let parts = &parts;
            sc.spawn(move || {
                let mut all = Names::new(groups / 64 + 1);
                for (_, names) in parts {
                    for (id, &h) in names.hashes.iter().enumerate() {
                        if (h as u128 * nt as u128 >> 64) as usize == shard {
                            let src = &names.masks[id * names.words..(id + 1) * names.words];
                            for (w, s) in all.mask(names.names[id], h).iter_mut().zip(src) {
                                *w |= s;
                            }
                        }
                    }
                }
                (0..groups).map(|g| all.masks.chunks(all.words).filter(|m| m[g / 64] >> (g % 64) & 1 == 1).count() as i64).collect()
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut o = Vec::new();
    for g in 0..groups {
        let mut a = parts[0].0[g];
        for (aggs, _) in &parts[1..] {
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
        let key = if g == unmatched { "_UNMATCHED" } else { std::str::from_utf8(codes[g]).unwrap() };
        let names: i64 = distinct.iter().map(|c| c[g]).sum();
        writeln!(o, "{key} {} {} {} {} {} {names} {} {}", a.count, a.id_sum, money(a.amount_sum), money(a.min), money(a.max), a.name_bytes, a.note_bytes).unwrap();
    }
    std::fs::write(format!("{out_dir}/aggregates.txt"), o).unwrap();
}
