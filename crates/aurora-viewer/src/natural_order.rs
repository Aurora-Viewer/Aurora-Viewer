//! Dictionary order of the names shown in lists: case ignored, and runs of
//! digits compared by their value, so "Pose 2" comes before "Pose 10". The
//! Legacy Viewer sorts its lists, outfits, groups and favourites the same way
//! (LLStringUtil::compareDict, indra/llcommon/llstring.h).

use std::cmp::Ordering;

/// Compares two names in dictionary order, case ignored.
pub fn cmp(a: &str, b: &str) -> Ordering {
    cmp_folded(&a.to_lowercase(), &b.to_lowercase())
}

/// Sort key of a name for `sort_by_key` / `sort_by_cached_key`: the name in
/// lower case, ordered by [`cmp_folded`].
pub fn key(name: &str) -> Key {
    Key(name.to_lowercase())
}

/// A name already in lower case, ordered in dictionary order. Two keys are
/// equal only when their text is, as `cmp_folded` returns `Equal` only then.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Key(String);

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_folded(&self.0, &other.0)
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Dictionary order of two names already folded to one case: numeric runs
/// compare numerically ("2" before "10"); with the same value, the run with
/// fewer leading zeros comes first ("1" before "01").
pub fn cmp_folded(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while let (Some(&x), Some(&y)) = (a.first(), b.first()) {
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (an, bn) = (
                a.iter().take_while(|c| c.is_ascii_digit()).count(),
                b.iter().take_while(|c| c.is_ascii_digit()).count(),
            );
            let (av, bv) = (&a[..an], &b[..bn]);
            let (az, bz) = (
                av.iter().take_while(|c| **c == b'0').count(),
                bv.iter().take_while(|c| **c == b'0').count(),
            );
            let order = (an - az)
                .cmp(&(bn - bz))
                .then_with(|| av[az..].cmp(&bv[bz..]))
                .then_with(|| an.cmp(&bn));
            if order != Ordering::Equal {
                return order;
            }
            a = &a[an..];
            b = &b[bn..];
        } else {
            let order = x.cmp(&y);
            if order != Ordering::Equal {
                return order;
            }
            a = &a[1..];
            b = &b[1..];
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sort_by_value() {
        let mut names = vec!["Pose 10", "pose 2", "Pose 1", "Pose 19", "Pose 3", "Pose 29", "Pose 20"];
        names.sort_by(|a, b| cmp(a, b));
        assert_eq!(names, ["Pose 1", "pose 2", "Pose 3", "Pose 10", "Pose 19", "Pose 20", "Pose 29"]);
    }

    #[test]
    fn key_matches_cmp() {
        let mut names = vec!["b12", "B3", "a", "b03", "B 1"];
        names.sort_by_cached_key(|n| key(n));
        assert_eq!(names, ["a", "B 1", "B3", "b03", "b12"]);
        assert_eq!(key("Abc"), key("aBC"));
        assert_ne!(key("1"), key("01"));
    }

    #[test]
    fn text_and_prefixes() {
        assert!(cmp_folded("folder 2", "folder 10").is_lt());
        assert!(cmp_folded("abc", "abcd").is_lt());
        assert!(cmp_folded("1", "01").is_lt());
        assert!(cmp_folded("9", "10").is_lt());
        assert!(cmp_folded("x99999999999999999999999", "x100000000000000000000000").is_lt());
        assert!(cmp("Élan", "élan").is_eq());
    }
}
