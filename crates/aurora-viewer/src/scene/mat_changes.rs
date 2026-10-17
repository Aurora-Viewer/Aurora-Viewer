//! Which objects a material change rebuilds.
//!
//! A material arriving (glTF asset, legacy material of a region, or one
//! that will never load) bumps the material generation. The faces built
//! with that material, or with the default while it loaded, must be built
//! again; the other objects using materials did not change. Rebuilding
//! every material user at each new material made bursts of full syncs in a
//! busy place: at KB (2026-10-16), ~135 objects rebuilt per frame for a few
//! seconds every time someone's materials arrived, `sync` at 8 to 10 ms.
//! Each changed material now notes the generation it changed at, and an
//! object is stale only when one of its own materials changed after it was
//! built.

use std::collections::HashMap;
use uuid::Uuid;

/// AURORA_MATERIAL_REBUILD: `all` rebuilds every material user at each
/// material change, as before; `alt` alternates the two ways by the second
/// (one second in three the old way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Targeted,
    All,
    Alternate,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_MATERIAL_REBUILD").as_deref() {
        Ok("all") => Mode::All,
        Ok("alt") => Mode::Alternate,
        _ => Mode::Targeted,
    })
}

static ALL_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this second rebuilds every material user (the old way).
pub fn all_now() -> bool {
    match mode() {
        Mode::All => true,
        Mode::Targeted => false,
        Mode::Alternate => ALL_NOW.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// `AURORA_MATERIAL_REBUILD=alt`: the way of second `n` (true: the old
/// way). None without `alt`.
pub fn alternate(n: u64) -> Option<bool> {
    (mode() == Mode::Alternate).then(|| {
        let old = n.is_multiple_of(3);
        ALL_NOW.store(old, std::sync::atomic::Ordering::Relaxed);
        old
    })
}

/// The generation at which each material last changed.
#[derive(Default)]
pub struct MatChanges {
    at: HashMap<Uuid, u64>,
}

impl MatChanges {
    /// Material `id` changed; `generation` is the material generation once
    /// the change is counted.
    pub fn note(&mut self, id: Uuid, generation: u64) {
        self.at.insert(id, generation);
    }

    /// Whether faces built at generation `built` with `ids` must be built
    /// again, the generation being `current` now.
    pub fn stale(&self, ids: &[Uuid], built: u64, current: u64) -> bool {
        if ids.is_empty() || built == current {
            return false;
        }
        if all_now() {
            return true;
        }
        ids.iter().any(|id| self.at.get(id).is_some_and(|&g| g > built))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_users_of_a_changed_material_are_stale() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut c = MatChanges::default();
        // built at generation 3, then material b arrives (generation 4)
        c.note(b, 4);
        assert!(!c.stale(&[a], 3, 4), "an object using a only is not rebuilt");
        assert!(c.stale(&[a, b], 3, 4), "an object using b is rebuilt");
        // built after the arrival: up to date
        assert!(!c.stale(&[b], 4, 4));
        assert!(!c.stale(&[b], 4, 5), "a later change of another material");
        // no materials: never stale
        assert!(!c.stale(&[], 0, 9));
    }

    #[test]
    fn an_older_change_does_not_rebuild_again() {
        let a = Uuid::from_u128(7);
        let mut c = MatChanges::default();
        c.note(a, 2);
        assert!(c.stale(&[a], 1, 2));
        assert!(!c.stale(&[a], 2, 6), "built after a's change: other materials changed since");
    }
}
