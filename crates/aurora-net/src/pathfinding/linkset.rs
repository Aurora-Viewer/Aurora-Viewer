//! How a linkset takes part in the navmesh, and the rules of its edit.
//! Port of LLPathfindingLinkset (indra/newview/llpathfindinglinkset.cpp) and
//! the selection rules of LLPathfindingLinksetList
//! (indra/newview/llpathfindinglinksetlist.cpp), originally LGPL 2.1.
use aurora_llsd::Llsd;
use uuid::Uuid;

/// The walkability coefficients' range (MIN/MAX_WALKABILITY_VALUE).
pub const MIN_WALKABILITY: u8 = 0;
pub const MAX_WALKABILITY: u8 = 100;

/// `navmesh_category` on the wire (LINKSET_CATEGORY_VALUE_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Include,
    Exclude,
    Ignore,
}

impl Category {
    pub fn from_wire(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Include),
            1 => Some(Self::Exclude),
            2 => Some(Self::Ignore),
            _ => None,
        }
    }

    pub fn wire(self) -> i32 {
        match self {
            Self::Include => 0,
            Self::Exclude => 1,
            Self::Ignore => 2,
        }
    }
}

/// ELinksetUse, the "Pathfinding use" column, without kUnknown: "no use
/// chosen" is an `Option<Use>` where it can happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Use {
    Walkable,
    StaticObstacle,
    MovableObstacle,
    MaterialVolume,
    ExclusionVolume,
    MovablePhantom,
}

impl Use {
    /// The order of the Legacy Viewer's use lists.
    pub const ALL: [Use; 6] = [
        Use::Walkable,
        Use::StaticObstacle,
        Use::MovableObstacle,
        Use::MaterialVolume,
        Use::ExclusionVolume,
        Use::MovablePhantom,
    ];

    /// LLPathfindingLinkset::getLinksetUse.
    pub fn new(phantom: bool, category: Category) -> Self {
        match (phantom, category) {
            (false, Category::Include) => Self::Walkable,
            (false, Category::Exclude) => Self::StaticObstacle,
            (false, Category::Ignore) => Self::MovableObstacle,
            (true, Category::Include) => Self::MaterialVolume,
            (true, Category::Exclude) => Self::ExclusionVolume,
            (true, Category::Ignore) => Self::MovablePhantom,
        }
    }

    pub fn is_phantom(self) -> bool {
        matches!(self, Self::MaterialVolume | Self::ExclusionVolume | Self::MovablePhantom)
    }

    /// getNavMeshGenerationCategory.
    pub fn category(self) -> Category {
        match self {
            Self::Walkable | Self::MaterialVolume => Category::Include,
            Self::StaticObstacle | Self::ExclusionVolume => Category::Exclude,
            Self::MovableObstacle | Self::MovablePhantom => Category::Ignore,
        }
    }

    pub fn is_volume(self) -> bool {
        matches!(self, Self::MaterialVolume | Self::ExclusionVolume)
    }

    /// getLinksetUseWithToggledPhantom: what a linkset that cannot change
    /// its phantom flag gets instead (the RESTRICTED_TYPE of the warning).
    pub fn with_toggled_phantom(self) -> Self {
        Self::new(!self.is_phantom(), self.category())
    }
}

/// One linkset's pathfinding state, as the region reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Linkset {
    pub use_: Use,
    /// A (humanoid), B (creature), C (mechanical), D (other), 0..=100.
    pub walkability: [u8; 4],
    /// The agent may change the phantom flag (`modifiable`).
    pub modifiable: bool,
    /// It may become a material or exclusion volume (`can_be_volume`).
    pub can_be_volume: bool,
    /// The region's terrain (TerrainNavMeshProperties), not an object.
    pub terrain: bool,
}

/// The changed fields of one linkset, for ObjectNavMeshProperties or
/// TerrainNavMeshProperties (encodeAlteredFields).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edit {
    pub category: Option<Category>,
    pub phantom: Option<bool>,
    pub walkability: [Option<u8>; 4],
}

impl Edit {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn to_llsd(&self) -> Llsd {
        let mut map = Llsd::new_map();
        if let Some(phantom) = self.phantom {
            map.insert("phantom", phantom);
        }
        if let Some(category) = self.category {
            map.insert("navmesh_category", category.wire());
        }
        for (field, value) in ["A", "B", "C", "D"].into_iter().zip(self.walkability) {
            if let Some(value) = value {
                map.insert(field, i32::from(value));
            }
        }
        map
    }
}

impl Linkset {
    /// encodeAlteredFields: only what changes. The use is sent when it
    /// differs and is allowed (a non-volume linkset never becomes a
    /// volume, the terrain never changes use), the phantom flag only when the
    /// agent may change it; each coefficient that differs, clamped.
    pub fn edit(&self, use_: Option<Use>, walkability: [i32; 4]) -> Edit {
        let mut edit = Edit::default();
        if let Some(use_) = use_
            && !self.terrain
            && use_ != self.use_
            && (self.can_be_volume || !use_.is_volume())
        {
            if self.modifiable {
                edit.phantom = Some(use_.is_phantom());
            }
            edit.category = Some(use_.category());
        }
        for (i, value) in walkability.into_iter().enumerate() {
            if value != i32::from(self.walkability[i]) {
                edit.walkability[i] = Some(value.clamp(MIN_WALKABILITY.into(), MAX_WALKABILITY.into()) as u8);
            }
        }
        edit
    }

    /// isShowPhantomToggleWarning.
    fn warns_phantom_toggle(&self, use_: Use) -> bool {
        self.modifiable && self.use_.is_phantom() != use_.is_phantom()
    }

    /// isShowUnmodifiablePhantomWarning.
    fn warns_restricted(&self, use_: Use) -> bool {
        !self.modifiable && self.use_.is_phantom() != use_.is_phantom()
    }

    /// isShowCannotBeVolumeWarning.
    fn warns_cannot_be_volume(&self, use_: Use) -> bool {
        !self.can_be_volume && use_.is_volume()
    }
}

/// determinePossibleStates: which uses the dropdown offers for a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Possible([bool; 6]);

impl Possible {
    pub fn of<'a>(selection: impl IntoIterator<Item = &'a Linkset>) -> Self {
        let mut allowed = Self::default();
        for linkset in selection {
            let uses: &[Use] = if linkset.terrain {
                &[Use::Walkable]
            } else if linkset.modifiable {
                &[Use::Walkable, Use::StaticObstacle, Use::MovableObstacle, Use::MovablePhantom]
            } else if linkset.use_.is_phantom() {
                &[Use::MovablePhantom]
            } else {
                &[Use::Walkable, Use::StaticObstacle, Use::MovableObstacle]
            };
            for use_ in uses {
                allowed.set(*use_);
            }
            if !linkset.terrain && (linkset.modifiable || linkset.use_.is_phantom()) && linkset.can_be_volume {
                allowed.set(Use::MaterialVolume);
                allowed.set(Use::ExclusionVolume);
            }
        }
        allowed
    }

    fn set(&mut self, use_: Use) {
        if let Some(i) = Use::ALL.iter().position(|u| *u == use_) {
            self.0[i] = true;
        }
    }

    pub fn allows(&self, use_: Use) -> bool {
        Use::ALL.iter().position(|u| *u == use_).is_some_and(|i| self.0[i])
    }
}

/// The confirmation `applyEdit` asks before sending, when any selected
/// linkset would toggle its phantom flag, cannot toggle it (it gets the
/// restricted use instead), or cannot become a volume. The Legacy Viewer
/// builds the notification's name from these three flags
/// (PathfindingLinksets_WarnOnPhantom_MismatchOnRestricted_MismatchOnVolume).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Warnings {
    pub phantom_toggle: bool,
    pub restricted: bool,
    pub cannot_be_volume: bool,
}

impl Warnings {
    pub fn of<'a>(selection: impl IntoIterator<Item = &'a Linkset>, use_: Option<Use>) -> Self {
        let Some(use_) = use_ else {
            return Self::default();
        };
        let mut warnings = Self::default();
        for linkset in selection {
            warnings.phantom_toggle |= linkset.warns_phantom_toggle(use_);
            warnings.restricted |= linkset.warns_restricted(use_);
            warnings.cannot_be_volume |= linkset.warns_cannot_be_volume(use_);
        }
        warnings
    }

    pub fn any(&self) -> bool {
        self.phantom_toggle || self.restricted || self.cannot_be_volume
    }
}

/// onWalkabilityCoefficientEntered: what a coefficient field holds after
/// an entry. Empty keeps the previous value, a number is clamped to
/// 0..=100, anything else becomes 100.
pub fn coefficient_entry(text: &str, previous: u8) -> u8 {
    let text = text.trim();
    if text.is_empty() {
        return previous;
    }
    match text.parse::<i32>() {
        Ok(value) => value.clamp(MIN_WALKABILITY.into(), MAX_WALKABILITY.into()) as u8,
        Err(_) => MAX_WALKABILITY,
    }
}

/// encodeObjectFields: the ObjectNavMeshProperties body, `{ <id>: { … } }`
/// for every object linkset with a change, or None when nothing changes.
pub fn objects_body(edits: &[(Uuid, Edit)]) -> Option<Llsd> {
    let mut map = Llsd::new_map();
    for (id, edit) in edits {
        if !edit.is_empty() {
            map.insert(id.to_string(), edit.to_llsd());
        }
    }
    map.as_map().is_some_and(|m| !m.is_empty()).then_some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linkset(use_: Use, modifiable: bool, can_be_volume: bool) -> Linkset {
        Linkset {
            use_,
            walkability: [100; 4],
            modifiable,
            can_be_volume,
            terrain: false,
        }
    }

    fn terrain() -> Linkset {
        Linkset {
            terrain: true,
            ..linkset(Use::Walkable, false, false)
        }
    }

    #[test]
    fn use_and_wire_fields_map_both_ways() {
        for use_ in Use::ALL {
            assert_eq!(Use::new(use_.is_phantom(), use_.category()), use_);
            assert_eq!(Category::from_wire(use_.category().wire()), Some(use_.category()));
        }
        assert_eq!(Category::from_wire(3), None);
        assert_eq!(Use::Walkable.with_toggled_phantom(), Use::MaterialVolume);
        assert_eq!(Use::ExclusionVolume.with_toggled_phantom(), Use::StaticObstacle);
        assert_eq!(Use::MovableObstacle.with_toggled_phantom(), Use::MovablePhantom);
    }

    #[test]
    fn edits_carry_only_what_changes() {
        let solid = linkset(Use::MovableObstacle, true, true);
        assert!(solid.edit(Some(Use::MovableObstacle), [100; 4]).is_empty());
        assert!(solid.edit(None, [100; 4]).is_empty());

        let edit = solid.edit(Some(Use::Walkable), [100, 50, 100, 100]);
        assert_eq!(edit.category, Some(Category::Include));
        assert_eq!(edit.phantom, Some(false));
        assert_eq!(edit.walkability, [None, Some(50), None, None]);
        assert_eq!(edit.to_llsd()["navmesh_category"].as_i32(), 0);
        assert_eq!(edit.to_llsd()["B"].as_i32(), 50);
        assert!(edit.to_llsd()["A"].is_undef());
    }

    #[test]
    fn coefficients_are_clamped_on_the_wire() {
        let edit = linkset(Use::Walkable, true, true).edit(None, [-5, 250, 100, 0]);
        assert_eq!(edit.walkability, [Some(0), Some(100), None, Some(0)]);
    }

    #[test]
    fn phantom_is_sent_only_when_modifiable() {
        let locked = linkset(Use::StaticObstacle, false, true);
        let edit = locked.edit(Some(Use::ExclusionVolume), [100; 4]);
        assert_eq!(edit.phantom, None);
        assert_eq!(edit.category, Some(Category::Exclude));
    }

    #[test]
    fn a_non_volume_never_becomes_a_volume_and_terrain_never_changes_use() {
        let no_volume = linkset(Use::Walkable, true, false);
        assert!(no_volume.edit(Some(Use::MaterialVolume), [100; 4]).is_empty());
        assert_eq!(
            no_volume.edit(Some(Use::StaticObstacle), [100; 4]).category,
            Some(Category::Exclude)
        );
        let ground = terrain();
        assert!(ground.edit(Some(Use::StaticObstacle), [100; 4]).category.is_none());
        assert_eq!(ground.edit(Some(Use::StaticObstacle), [40, 100, 100, 100]).walkability[0], Some(40));
    }

    #[test]
    fn possible_uses_follow_each_kind_of_linkset() {
        let only = |l: Linkset| {
            let possible = Possible::of([&l]);
            Use::ALL.into_iter().filter(|u| possible.allows(*u)).collect::<Vec<_>>()
        };
        assert_eq!(only(terrain()), [Use::Walkable]);
        assert_eq!(only(linkset(Use::Walkable, true, true)), Use::ALL);
        assert_eq!(
            only(linkset(Use::Walkable, true, false)),
            [Use::Walkable, Use::StaticObstacle, Use::MovableObstacle, Use::MovablePhantom]
        );
        assert_eq!(
            only(linkset(Use::MovablePhantom, false, true)),
            [Use::MaterialVolume, Use::ExclusionVolume, Use::MovablePhantom]
        );
        assert_eq!(only(linkset(Use::MovablePhantom, false, false)), [Use::MovablePhantom]);
        assert_eq!(
            only(linkset(Use::StaticObstacle, false, true)),
            [Use::Walkable, Use::StaticObstacle, Use::MovableObstacle]
        );
        assert_eq!(
            only(linkset(Use::Walkable, false, true)),
            only(linkset(Use::Walkable, false, false))
        );
        let mixed = [linkset(Use::MovablePhantom, false, false), terrain()];
        let possible = Possible::of(&mixed);
        assert!(possible.allows(Use::Walkable) && possible.allows(Use::MovablePhantom));
        assert!(!possible.allows(Use::StaticObstacle));
        assert_eq!(Possible::of(&[]), Possible::default());
    }

    #[test]
    fn warnings_combine_over_the_selection() {
        let selection = [linkset(Use::Walkable, true, true), linkset(Use::StaticObstacle, false, false)];
        assert_eq!(
            Warnings::of(&selection, Some(Use::MaterialVolume)),
            Warnings {
                phantom_toggle: true,
                restricted: true,
                cannot_be_volume: true
            }
        );
        assert!(!Warnings::of(&selection, Some(Use::MovableObstacle)).any());
        assert!(!Warnings::of(&selection, None).any());
        let only_toggle = Warnings::of(&selection[..1], Some(Use::MovablePhantom));
        assert!(only_toggle.phantom_toggle && !only_toggle.restricted && !only_toggle.cannot_be_volume);
    }

    #[test]
    fn coefficient_entries_follow_the_legacy_field() {
        assert_eq!(coefficient_entry("", 42), 42);
        assert_eq!(coefficient_entry("  ", 42), 42);
        assert_eq!(coefficient_entry("55", 42), 55);
        assert_eq!(coefficient_entry("-3", 42), 0);
        assert_eq!(coefficient_entry("300", 42), 100);
        assert_eq!(coefficient_entry("abc", 42), 100);
    }

    #[test]
    fn the_objects_body_keeps_only_changed_linksets() {
        let changed = linkset(Use::Walkable, true, true).edit(Some(Use::StaticObstacle), [100; 4]);
        let id = Uuid::from_u128(5);
        let body = objects_body(&[(id, changed), (Uuid::from_u128(6), Edit::default())]).expect("a change");
        let map = body.as_map().expect("a map");
        assert_eq!(map.len(), 1);
        assert_eq!(body[id.to_string().as_str()]["navmesh_category"].as_i32(), 1);
        assert!(objects_body(&[(id, Edit::default())]).is_none());
        assert!(objects_body(&[]).is_none());
    }
}
