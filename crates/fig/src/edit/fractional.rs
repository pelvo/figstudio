//! Byte-lexicographic fractional indexing for Figma `parentIndex.position`.
//!
//! Figma sorts sibling `position` strings by raw ASCII byte order (`'!'` = 33 .. `'~'` = 126)
//! and forbids trailing `'!'` (the minimum digit, analogous to trailing zeros after a decimal
//! point) so there is always strictly another string before any valid key.

const MIN_CHAR: u8 = b'!'; // 33
const MAX_CHAR: u8 = b'~'; // 126
const MID_CHAR: u8 = b'N'; // 78, roughly halfway between '!' and '~'

/// Returns a printable ASCII string strictly between `prev` and `next` in byte order.
///
/// If `prev >= next` (degenerate/equal existing positions) or `next` is a malformed key that no
/// valid key precedes, the result is not between them; use [`try_between`] to detect that.
/// Appending (`next == None`) always succeeds.
pub fn between(prev: Option<&str>, next: Option<&str>) -> String {
    let a: Vec<u8> = prev.unwrap_or("").bytes().map(clamp_digit).collect();
    let b: Vec<u8> = next.unwrap_or("").bytes().map(clamp_digit).collect();
    if !a.is_empty() && !b.is_empty() && a >= b {
        return after_bytes(&a);
    }
    match (a.is_empty(), b.is_empty()) {
        (true, true) => String::from_utf8(vec![MID_CHAR]).unwrap_or_else(|_| "N".into()),
        (false, true) => after_bytes(&a),
        (true, false) => before_bytes(&b),
        (false, false) => midpoint_bytes(&a, &b),
    }
}

/// [`between`], or `None` when no valid key fits: `prev >= next`, or `next` is a malformed key
/// that only `'!'`s could precede (such as `"!"`). A returned key is inside the alphabet, does
/// not end in `'!'`, and sorts strictly between `prev` and `next`.
pub fn try_between(prev: Option<&str>, next: Option<&str>) -> Option<String> {
    let key = between(prev, next);
    let valid = key.bytes().all(|b| (MIN_CHAR..=MAX_CHAR).contains(&b))
        && key.as_bytes().last().is_some_and(|&b| b != MIN_CHAR)
        && prev.is_none_or(|p| p.as_bytes() < key.as_bytes())
        && next.is_none_or(|n| key.as_bytes() < n.as_bytes());
    valid.then_some(key)
}

/// Generates `count` strictly ascending position keys between `prev` and `next`.
pub fn n_between(prev: Option<&str>, next: Option<&str>, count: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(count);
    let mut cur_prev: Option<String> = prev.map(str::to_owned);
    for _ in 0..count {
        let key = between(cur_prev.as_deref(), next);
        cur_prev = Some(key.clone());
        out.push(key);
    }
    out
}

fn clamp_digit(b: u8) -> u8 {
    b.clamp(MIN_CHAR, MAX_CHAR)
}

fn after_bytes(a: &[u8]) -> String {
    let mut out = Vec::with_capacity(a.len() + 1);
    for (i, &raw) in a.iter().enumerate() {
        let c = clamp_digit(raw);
        if i + 1 == a.len() && c < MAX_CHAR - 1 {
            let step = ((u16::from(c) + u16::from(MAX_CHAR)) / 2) as u8;
            if step > c {
                out.push(step);
                return String::from_utf8(out).unwrap_or_else(|_| "N".into());
            }
        }
        out.push(c);
    }
    out.push(MID_CHAR);
    String::from_utf8(out).unwrap_or_else(|_| "N".into())
}

fn before_bytes(b: &[u8]) -> String {
    let mut out = Vec::with_capacity(b.len() + 2);
    for &raw in b {
        if raw <= MIN_CHAR {
            out.push(MIN_CHAR);
            continue;
        }
        let hi = raw.min(MAX_CHAR);
        let mid = ((u16::from(MIN_CHAR) + u16::from(hi)) / 2) as u8;
        if mid > MIN_CHAR && mid < raw {
            out.push(mid);
            return String::from_utf8(out).unwrap_or_else(|_| "N".into());
        }
        out.push(MIN_CHAR);
        out.push(MID_CHAR);
        return String::from_utf8(out).unwrap_or_else(|_| "!N".into());
    }
    // `b` consisted solely of bytes <= MIN_CHAR (e.g. a malformed existing `"!"` key):
    // stay strictly inside the documented `'!'..='~'` alphabet instead of returning `' '` (0x20).
    String::from("!")
}

fn midpoint_bytes(a: &[u8], b: &[u8]) -> String {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 2);
    let mut i = 0;
    loop {
        let ca = a.get(i).copied().unwrap_or(MIN_CHAR);
        let cb = b.get(i).copied().unwrap_or(MAX_CHAR + 1);
        if ca == cb {
            out.push(ca);
            i += 1;
            continue;
        }
        if cb > ca + 1 {
            let hi = cb.min(MAX_CHAR);
            let mid = ((u16::from(ca.max(MIN_CHAR)) + u16::from(hi)) / 2) as u8;
            if mid > ca && mid < cb {
                out.push(mid);
                return String::from_utf8(out).unwrap_or_else(|_| "N".into());
            }
        }
        // Adjacent bytes (`cb == ca + 1`): emit `ca` and find a suffix strictly > a[i+1..].
        out.push(ca);
        let tail = after_bytes(a.get(i + 1..).unwrap_or(&[]));
        out.extend_from_slice(tail.as_bytes());
        return String::from_utf8(out).unwrap_or_else(|_| "N".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strictly_ordered_across_insertions() {
        let first = between(None, None);
        let before = between(None, Some(&first));
        let after = between(Some(&first), None);
        assert!(before.as_bytes() < first.as_bytes());
        assert!(first.as_bytes() < after.as_bytes());

        // 200 repeated bisections between two adjacent keys stay strictly ordered.
        let mut lo = String::from("!");
        let hi = String::from("\"");
        for _ in 0..200 {
            let mid = between(Some(&lo), Some(&hi));
            assert!(
                lo.as_bytes() < mid.as_bytes() && mid.as_bytes() < hi.as_bytes(),
                "failed bisection: lo={lo:?} mid={mid:?} hi={hi:?}"
            );
            lo = mid;
        }
    }

    #[test]
    fn try_between_finds_a_valid_key_for_every_ordered_pair_of_valid_keys() {
        // Keys as the editor makes them: appends, front inserts and bisections.
        let mut keys = vec![between(None, None)];
        for i in 0..60 {
            let k = match i % 3 {
                0 => between(keys.last().map(String::as_str), None),
                1 => between(None, Some(&keys[0])),
                _ => between(Some(&keys[0]), Some(&keys[1])),
            };
            keys.push(k);
            keys.sort();
            keys.dedup();
        }
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                let k = try_between(Some(a), Some(b));
                assert!(k.is_some(), "no key between {a:?} and {b:?}");
            }
            assert!(try_between(None, Some(a)).is_some(), "no key before {a:?}");
        }
    }

    #[test]
    fn try_between_rejects_inputs_with_no_valid_key_between() {
        assert_eq!(try_between(None, Some("!")), None);
        assert_eq!(try_between(None, Some("!!")), None);
        assert_eq!(try_between(Some("B"), Some("A")), None);
        assert_eq!(try_between(Some("A"), Some("A")), None);
    }

    #[test]
    fn batch_generation_is_strictly_ascending() {
        let keys = n_between(Some("A"), Some("B"), 32);
        assert_eq!(keys.len(), 32);
        assert!("A".as_bytes() < keys[0].as_bytes());
        assert!(keys.last().unwrap().as_bytes() < "B".as_bytes());
        for pair in keys.windows(2) {
            assert!(pair[0].as_bytes() < pair[1].as_bytes());
        }
    }
}
