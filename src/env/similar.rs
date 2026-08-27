//! Is one variable name a misspelling of another.
//!
//! 🔴 Plain edit distance produced three false findings out of three on live
//! projects, all of them pairs that are simply two different settings:
//!
//! ```text
//! SHORTIFY_UVICORN_HOST  / SHORTIFY_UVICORN_PORT   (2 edits)
//! API_WORKER_ID          / API_WORKERS             (3 edits)
//! ..._MTLS_CERT_FILE     / ..._MTLS_CA_CERT_FILE   (an inserted segment)
//! ```
//!
//! A typo does not add or remove a segment and does not rewrite one whole.
//! Hence two rules, and no threshold picked to make one example pass.

/// Pairs where the last segment differs but the meaning does not. These are
/// never caught by distance and are common enough to list by hand.
const TAIL_SYNONYMS: &[&[&str]] = &[
    &["host", "hostname"],
    &["user", "username"],
    &["pass", "passwd", "password"],
    &["pwd", "password"],
    &["url", "uri"],
    &["addr", "address"],
    &["db", "database"],
    &["dir", "directory", "path"],
    &["key", "secret"],
    &["num", "number", "count"],
];

fn normalise(s: &str) -> String {
    s.to_lowercase().replace(['_', '-'], "")
}

fn segments(s: &str) -> Vec<String> {
    s.to_lowercase().split('_').map(String::from).collect()
}

/// Levenshtein distance, bounded: gives up as soon as it exceeds `k`.
fn edit_within(a: &str, b: &str, k: usize) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > k {
        return false;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost));
        }
        if cur.iter().min().copied().unwrap_or(0) > k {
            return false;
        }
        prev = cur;
    }
    prev[b.len()] <= k
}

/// Same name up to a synonymous last segment: SMTP_HOST / SMTP_HOSTNAME.
fn tail_synonym(a: &str, b: &str) -> bool {
    let (pa, pb) = (segments(a), segments(b));
    if pa.len() != pb.len() || pa[..pa.len() - 1] != pb[..pb.len() - 1] {
        return false;
    }
    let (ta, tb) = (pa.last().unwrap(), pb.last().unwrap());
    if ta == tb {
        return false;
    }
    TAIL_SYNONYMS
        .iter()
        .any(|group| group.contains(&ta.as_str()) && group.contains(&tb.as_str()))
}

/// A typo inside one segment, everything else identical.
fn one_segment_typo(a: &str, b: &str) -> bool {
    let (pa, pb) = (segments(a), segments(b));
    if pa.len() != pb.len() {
        return false;
    }
    let diff: Vec<usize> = pa
        .iter()
        .zip(pb.iter())
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .map(|(i, _)| i)
        .collect();
    if diff.len() != 1 {
        return false;
    }
    let (x, y) = (&pa[diff[0]], &pb[diff[0]]);
    // A short segment replaced whole is a different setting, not a typo:
    // HOST against PORT is four letters swapped for four others.
    if x.len() < 5 || y.len() < 5 {
        return x.len().abs_diff(y.len()) <= 2 && edit_within(x, y, 1);
    }
    edit_within(x, y, 2)
}

/// The name in `pool` that `name` is most likely a misspelling of.
pub fn similar<'a, I: IntoIterator<Item = &'a String>>(name: &str, pool: I) -> Option<String> {
    let n = normalise(name);
    for other in pool {
        if other == name {
            continue;
        }
        if normalise(other) == n || tail_synonym(name, other) || one_segment_typo(name, other) {
            return Some(other.clone());
        }
    }
    None
}
