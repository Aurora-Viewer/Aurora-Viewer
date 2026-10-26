//! Photo isolation (issue #977; an Aurora addition, the Legacy Viewer has
//! none): the photographer paints over avatars and objects to mark them,
//! then either hides what is marked or shows nothing else. A mark is a
//! whole avatar with what it wears, or a whole linkset: what the brush
//! touches is followed up to its root. With "show only", the sky, the
//! terrain, the water and the clouds are not drawn either, and the photo
//! keeps its alpha: a PNG with nothing behind the subject.
//!
//! The rule is read by `World::parcel_object_visibility`, the one scope
//! the scene, the lights, the particles and the labels already follow for
//! the parcel filter. While a brush is in hand everything is drawn, the
//! marked objects outlined, so that what is hidden can be painted back.

use std::collections::HashSet;
use uuid::Uuid;

/// What the marks do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Everything drawn, marks kept for later.
    #[default]
    Off,
    /// The marked avatars and linksets are not drawn.
    Hide,
    /// Only the marked ones are drawn.
    Only,
}

/// The brush in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brush {
    Add,
    Remove,
}

/// The isolation of the photo session.
#[derive(Debug, Clone, PartialEq)]
pub struct Isolation {
    pub mode: Mode,
    /// The roots marked (an avatar, or the root prim of a linkset).
    marked: HashSet<Uuid>,
    /// "Show only" leaves the background empty: no sky, terrain, water or
    /// clouds, and the photo's alpha is kept.
    pub transparent: bool,
    /// A brush in hand: everything drawn, the marks outlined.
    pub brush: Option<Brush>,
    /// The photo mode is open: outside it everything is drawn, the mode and
    /// the marks kept for its next opening.
    pub open: bool,
    /// Bumped when what is drawn changes: the scene syncs every object
    /// again (as for the parcel filter).
    pub version: u64,
    /// Bumped when the marks change: their outline only.
    pub marks_version: u64,
}

impl Default for Isolation {
    fn default() -> Self {
        Isolation {
            mode: Mode::Off,
            marked: HashSet::new(),
            transparent: true,
            brush: None,
            open: false,
            version: 0,
            marks_version: 0,
        }
    }
}

impl Isolation {
    /// The marks decide what is drawn now.
    pub fn applied(&self) -> bool {
        self.open && self.brush.is_none() && self.mode != Mode::Off
    }

    /// The background is left empty now (and the photo keeps its alpha).
    pub fn empty_background(&self) -> bool {
        self.applied() && self.mode == Mode::Only && self.transparent
    }

    /// Whether the avatar or linkset whose root is `root` is drawn.
    pub fn root_visible(&self, root: &Uuid) -> bool {
        if !self.applied() {
            return true;
        }
        match self.mode {
            Mode::Off => true,
            Mode::Hide => !self.marked.contains(root),
            Mode::Only => self.marked.contains(root),
        }
    }

    pub fn marked(&self) -> impl Iterator<Item = &Uuid> {
        self.marked.iter()
    }

    pub fn count(&self) -> usize {
        self.marked.len()
    }

    /// The brush passed over `root`; true when the marks changed.
    pub fn paint(&mut self, root: Uuid, brush: Brush) -> bool {
        let changed = match brush {
            Brush::Add => self.marked.insert(root),
            Brush::Remove => self.marked.remove(&root),
        };
        if changed {
            self.marks_changed();
        }
        changed
    }

    pub fn clear(&mut self) {
        if !self.marked.is_empty() {
            self.marked.clear();
            self.marks_changed();
        }
    }

    /// Forget the roots that left the scene (a teleport, an avatar gone).
    pub fn retain(&mut self, mut present: impl FnMut(&Uuid) -> bool) {
        let before = self.marked.len();
        self.marked.retain(|id| present(id));
        if self.marked.len() != before {
            self.marks_changed();
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            let applied = self.applied();
            self.mode = mode;
            if applied || self.applied() {
                self.bump();
            }
        }
    }

    /// (the renderer's switch alone: no object changes)
    pub fn set_transparent(&mut self, on: bool) {
        self.transparent = on;
    }

    pub fn set_brush(&mut self, brush: Option<Brush>) {
        if self.brush != brush {
            let applied = self.applied();
            self.brush = brush;
            if applied != self.applied() {
                self.bump();
            }
        }
    }

    /// The photo mode opens or closes (the brush is put down).
    pub fn set_open(&mut self, open: bool) {
        if self.open != open {
            let applied = self.applied();
            self.open = open;
            self.brush = None;
            if applied != self.applied() {
                self.bump();
            }
        }
    }

    fn marks_changed(&mut self) {
        self.marks_version = self.marks_version.wrapping_add(1);
        if self.applied() {
            self.bump();
        }
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
}

/// What a mark is: the avatar of what it wears, a linkset's root prim, the
/// avatar itself even when seated. None for a HUD (interface, never
/// isolated) or an object whose parent is not known yet.
pub fn root_of(world: &crate::world::World, index: usize) -> Option<(usize, Uuid)> {
    let mut index = index;
    for _ in 0..64 {
        let object = world.objects.get(index)?;
        if object.is_avatar() || object.parent_id == 0 {
            return Some((index, object.full_id));
        }
        // (the attachment point is the state of a worn root prim; a tree's
        // state is its species, hence the parent)
        if (31..=38).contains(&object.attachment_point())
            && world
                .objects
                .parent_of(object)
                .is_some_and(|p| world.objects.get(p).is_some_and(|a| a.is_avatar()))
        {
            return None;
        }
        index = world.objects.parent_of(object)?;
    }
    None
}

/// Whether the object at `index` is drawn under the photo's isolation.
pub fn object_visible(world: &crate::world::World, index: usize) -> bool {
    let iso = &world.photo_isolation;
    !iso.applied() || root_of(world, index).is_none_or(|(_, root)| iso.root_visible(&root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn hide_and_only_follow_the_marks_and_the_brush_suspends_them() {
        let mut iso = Isolation::default();
        iso.set_open(true);
        assert!(iso.root_visible(&id(1)), "off: everything drawn");
        assert!(iso.paint(id(1), Brush::Add));
        assert!(!iso.paint(id(1), Brush::Add), "painting twice changes nothing");
        iso.set_mode(Mode::Hide);
        assert!(!iso.root_visible(&id(1)));
        assert!(iso.root_visible(&id(2)));
        iso.set_mode(Mode::Only);
        assert!(iso.root_visible(&id(1)));
        assert!(!iso.root_visible(&id(2)));
        assert!(iso.empty_background());
        iso.set_brush(Some(Brush::Remove));
        assert!(iso.root_visible(&id(2)), "a brush in hand shows everything");
        assert!(!iso.empty_background());
        assert!(iso.paint(id(1), Brush::Remove));
        iso.set_brush(None);
        assert!(!iso.root_visible(&id(1)), "only, nothing marked: nothing drawn");
    }

    #[test]
    fn only_what_changes_the_drawing_bumps_the_version() {
        let mut iso = Isolation::default();
        iso.set_open(true);
        iso.set_mode(Mode::Off);
        iso.clear();
        iso.retain(|_| true);
        assert_eq!((iso.version, iso.marks_version), (0, 0), "no change, no new version");
        iso.paint(id(3), Brush::Add);
        assert_eq!((iso.version, iso.marks_version), (0, 1), "off: the marks alone");
        iso.set_mode(Mode::Hide);
        assert_eq!(iso.version, 1);
        iso.set_brush(Some(Brush::Add));
        iso.paint(id(4), Brush::Add);
        iso.set_brush(Some(Brush::Remove));
        assert_eq!(iso.version, 2, "painting with a brush in hand draws everything");
        iso.set_brush(None);
        iso.retain(|r| *r != id(3));
        assert_eq!(iso.version, 4);
        assert_eq!(iso.count(), 1);
        iso.set_open(false);
        assert!(
            iso.root_visible(&id(4)) && iso.root_visible(&id(5)),
            "photo mode closed: everything drawn"
        );
        assert_eq!(
            (iso.version, iso.mode, iso.count()),
            (5, Mode::Hide, 1),
            "the mode and the marks kept"
        );
    }
}
