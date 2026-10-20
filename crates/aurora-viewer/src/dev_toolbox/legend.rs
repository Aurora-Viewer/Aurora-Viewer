//! J1 / J2 name the same textures and materials on many faces, each time
//! with their loading state: on a mesh body that is most of the copy. Every
//! id met more than once is written `~n` instead, and a legend at the end
//! gives it once with the first state that followed it. The ids of
//! prims are kept: their line follows them, and they are what a developer
//! looks up.

use std::collections::{HashMap, HashSet};

const ID_LEN: usize = 36;

/// A hyphenated UUID at the start of `b`.
fn is_uuid(b: &[u8]) -> bool {
    b.len() >= ID_LEN
        && b[..ID_LEN].iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The " (…)" right after an id (balanced parentheses): its state.
fn state_after(s: &str) -> Option<&str> {
    let rest = s.strip_prefix(" (")?;
    let mut depth = 1;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..i + 3]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The ids of `text` with their byte offsets.
fn ids(text: &str) -> Vec<(usize, &str)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + ID_LEN <= b.len() {
        let starts = i == 0 || !(b[i - 1].is_ascii_hexdigit() || b[i - 1] == b'-');
        let ends = b.get(i + ID_LEN).is_none_or(|c| !(c.is_ascii_hexdigit() || *c == b'-'));
        if starts && ends && is_uuid(&b[i..]) {
            out.push((i, &text[i..i + ID_LEN]));
            i += ID_LEN;
        } else {
            i += 1;
        }
    }
    out
}

/// `text` with every id met more than once (prims' aside) written `~n`, its
/// first state left out wherever it repeats, and the legend
/// appended. A text without such ids comes back unchanged.
pub fn shorten(text: &str) -> String {
    let found = ids(text);
    let mut count: HashMap<&str, usize> = HashMap::new();
    let mut state: HashMap<&str, Option<&str>> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    let mut prims: HashSet<&str> = HashSet::new();
    for &(at, id) in &found {
        if text[..at].ends_with("prim ") {
            prims.insert(id);
        }
        let n = count.entry(id).or_default();
        if *n == 0 {
            order.push(id);
        }
        *n += 1;
        // the first state given (a bake's first mention has none)
        let first = state.entry(id).or_default();
        if first.is_none() {
            *first = state_after(&text[at + ID_LEN..]);
        }
    }
    let alias: HashMap<&str, usize> = order
        .iter()
        .filter(|id| count[*id] > 1 && !prims.contains(*id))
        .enumerate()
        .map(|(n, id)| (*id, n + 1))
        .collect();
    if alias.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for &(at, id) in &found {
        let Some(n) = alias.get(id) else {
            continue;
        };
        out.push_str(&text[from..at]);
        out.push_str(&format!("~{n}"));
        from = at + ID_LEN;
        let after = state_after(&text[from..]);
        if after.is_some() && after == state[id] {
            from += after.map_or(0, str::len);
        }
    }
    out.push_str(&text[from..]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("ids named more than once (~n: the id, then the first state given):\n");
    let mut legend: Vec<(&usize, &&str)> = alias.iter().map(|(id, n)| (n, id)).collect();
    legend.sort();
    for (n, id) in legend {
        out.push_str(&format!("~{n} {id}{}\n", state[*id].unwrap_or_default()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "2bd25806-c894-2b00-807e-495a276ddcd5";
    const M: &str = "14a464db-b250-f821-8e92-39a491ed4f2f";
    const P: &str = "884f1067-9eee-8d31-e7d2-8d2c794ae2a7";

    #[test]
    fn repeated_ids_get_a_legend() {
        let text = format!(
            "prim {P} (mesh): [0 tex {T} (392990 bytes (complete), decoded Some(0)) legacy {M}] [1 tex {T} (392990 bytes (complete), decoded Some(0)) legacy {M}]\nglTF face {P} #0: x\n"
        );
        let short = shorten(&text);
        assert_eq!(
            short,
            format!(
                "prim {P} (mesh): [0 tex ~1 legacy ~2] [1 tex ~1 legacy ~2]\nglTF face {P} #0: x\nids named more than once (~n: the id, then the first state given):\n~1 {T} (392990 bytes (complete), decoded Some(0))\n~2 {M}\n"
            )
        );
    }

    #[test]
    fn a_changed_state_is_kept() {
        let text = format!("tex {T} (0 bytes) then tex {T} (100 bytes (complete))");
        assert_eq!(shorten(&text).lines().next(), Some("tex ~1 then tex ~1 (100 bytes (complete))"));
    }

    #[test]
    fn single_ids_stay() {
        let text = format!("tex {T} (0 bytes) and {M}");
        assert_eq!(shorten(&text), text);
    }

    #[test]
    fn ids_inside_longer_hex_are_not_ids() {
        assert!(ids(&format!("a{T}")).is_empty());
        assert_eq!(ids(&format!("[{T}, {M}]")).len(), 2);
    }
}
