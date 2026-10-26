//! Photo isolation (issue #977; an Aurora addition, the Legacy Viewer has
//! none): the photo shows our own avatar alone, or the avatars and objects
//! the photographer chose, everything else left out. A choice is a whole
//! avatar with what it wears, or a whole linkset: what is clicked is
//! followed up to its root. With the transparent background (the default)
//! the sky, the terrain, the water and the clouds are not drawn either,
//! and the photo keeps its alpha: a PNG with nothing around the subject.
//!
//! The scene flags what is left out for the GPU lists
//! (`object_flags::PHOTO_HIDDEN`); the particles, name plates and water
//! effects read [`object_visible`]. While choosing in the scene everything
//! is drawn, the chosen ones outlined, so that anything can be clicked.

use uuid::Uuid;

/// What the photo shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Everything.
    #[default]
    Off,
    /// Our own avatar alone.
    Own,
    /// The avatars and objects chosen.
    Chosen,
}

/// The isolation of the photo session.
#[derive(Debug, Clone, PartialEq)]
pub struct Isolation {
    pub mode: Mode,
    /// The roots chosen (an avatar, or the root prim of a linkset), in the
    /// order they were chosen.
    chosen: Vec<Uuid>,
    /// Our own avatar (`Mode::Own`).
    own: Uuid,
    /// The background is left empty: no sky, terrain, water or clouds,
    /// and the photo's alpha is kept.
    pub transparent: bool,
    /// Choosing in the scene: everything drawn, the chosen outlined, a
    /// click adds or removes what it lands on.
    pub picking: bool,
    /// The photo mode is open: outside it everything is drawn, the mode and
    /// the choices kept for its next opening.
    pub open: bool,
    /// Bumped when what is drawn changes: the scene flags every object
    /// again.
    pub version: u64,
}

impl Default for Isolation {
    fn default() -> Self {
        Isolation {
            mode: Mode::Off,
            chosen: Vec::new(),
            own: Uuid::nil(),
            transparent: true,
            picking: false,
            open: false,
            version: 0,
        }
    }
}

impl Isolation {
    /// The choice decides what is drawn now.
    pub fn applied(&self) -> bool {
        self.open && !self.picking && self.mode != Mode::Off
    }

    /// The background is left empty now (and the photo keeps its alpha).
    pub fn empty_background(&self) -> bool {
        self.applied() && self.transparent
    }

    /// Whether the avatar or linkset whose root is `root` is drawn.
    pub fn root_visible(&self, root: &Uuid) -> bool {
        if !self.applied() {
            return true;
        }
        match self.mode {
            Mode::Off => true,
            Mode::Own => *root == self.own,
            Mode::Chosen => self.chosen.contains(root),
        }
    }

    pub fn is_chosen(&self, root: &Uuid) -> bool {
        self.chosen.contains(root)
    }

    pub fn chosen(&self) -> &[Uuid] {
        &self.chosen
    }

    /// A click on `root` while choosing: added, or removed when it was
    /// chosen. True when added.
    pub fn toggle(&mut self, root: Uuid) -> bool {
        let added = if let Some(i) = self.chosen.iter().position(|id| *id == root) {
            self.chosen.remove(i);
            false
        } else {
            self.chosen.push(root);
            true
        };
        self.choice_changed();
        added
    }

    pub fn remove(&mut self, root: &Uuid) {
        if let Some(i) = self.chosen.iter().position(|id| id == root) {
            self.chosen.remove(i);
            self.choice_changed();
        }
    }

    /// Forget the roots that left the scene (a teleport, an avatar gone).
    pub fn retain(&mut self, mut present: impl FnMut(&Uuid) -> bool) {
        let before = self.chosen.len();
        self.chosen.retain(|id| present(id));
        if self.chosen.len() != before {
            self.choice_changed();
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

    /// Our own avatar, known again at each frame (a relog changes it).
    pub fn set_own(&mut self, own: Uuid) {
        if self.own != own {
            self.own = own;
            if self.applied() && self.mode == Mode::Own {
                self.bump();
            }
        }
    }

    /// (the renderer's switch alone: no object changes)
    pub fn set_transparent(&mut self, on: bool) {
        self.transparent = on;
    }

    pub fn set_picking(&mut self, on: bool) {
        if self.picking != on {
            let applied = self.applied();
            self.picking = on;
            if applied != self.applied() {
                self.bump();
            }
        }
    }

    /// The photo mode opens or closes (the choosing stops).
    pub fn set_open(&mut self, open: bool) {
        if self.open != open {
            let applied = self.applied();
            self.open = open;
            self.picking = false;
            if applied != self.applied() {
                self.bump();
            }
        }
    }

    fn choice_changed(&mut self) {
        if self.applied() && self.mode == Mode::Chosen {
            self.bump();
        }
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
}

/// What a choice is: the avatar of what it wears, a linkset's root prim, the
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

    fn opened() -> Isolation {
        let mut iso = Isolation::default();
        iso.set_open(true);
        iso.set_own(id(9));
        iso
    }

    #[test]
    fn own_shows_our_avatar_alone_and_chosen_the_choices() {
        let mut iso = opened();
        assert!(iso.root_visible(&id(1)), "off: everything drawn");
        iso.set_mode(Mode::Own);
        assert!(iso.root_visible(&id(9)));
        assert!(!iso.root_visible(&id(1)));
        assert!(iso.empty_background());
        iso.set_mode(Mode::Chosen);
        assert!(!iso.root_visible(&id(9)), "chosen, nothing chosen: nothing drawn");
        assert!(iso.toggle(id(1)));
        assert!(iso.toggle(id(2)));
        assert!(iso.root_visible(&id(1)) && iso.root_visible(&id(2)));
        assert!(!iso.toggle(id(1)), "a second click removes");
        assert!(!iso.root_visible(&id(1)));
        assert_eq!(iso.chosen(), &[id(2)]);
        iso.set_picking(true);
        assert!(iso.root_visible(&id(1)), "choosing shows everything");
        assert!(!iso.empty_background());
        iso.set_transparent(false);
        iso.set_picking(false);
        assert!(!iso.empty_background(), "the sky and the ground kept");
    }

    #[test]
    fn only_what_changes_the_drawing_bumps_the_version() {
        let mut iso = opened();
        iso.toggle(id(3));
        iso.remove(&id(4));
        iso.retain(|_| true);
        assert_eq!(iso.version, 0, "off: the choices alone");
        iso.set_mode(Mode::Chosen);
        assert_eq!(iso.version, 1);
        iso.set_picking(true);
        iso.toggle(id(4));
        assert_eq!(iso.version, 2, "choosing draws everything: a click changes nothing drawn");
        iso.set_picking(false);
        iso.retain(|r| *r != id(3));
        assert_eq!(iso.version, 4);
        iso.set_mode(Mode::Own);
        iso.set_own(id(8));
        assert_eq!(iso.version, 6);
        iso.set_open(false);
        assert!(
            iso.root_visible(&id(4)) && iso.root_visible(&id(5)),
            "photo mode closed: everything drawn"
        );
        assert_eq!(
            (iso.version, iso.mode, iso.chosen()),
            (7, Mode::Own, &[id(4)][..]),
            "the mode and the choices kept"
        );
    }
}
