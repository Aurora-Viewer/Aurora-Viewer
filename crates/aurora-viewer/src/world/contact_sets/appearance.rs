//! Optional local presentation of contact sets (#408). Unlike the Legacy
//! Viewer's smallest-set colour rule, each enabled attribute cascades by
//! explicit priority. No server-side friendship rights are changed.

use super::{ContactSets, Llsd};
use std::collections::HashMap;
use uuid::Uuid;

/// Curated built-in Tabler assets; persisted by name, never by list position.
pub const ICONS: &[&str] = &[
    "heart-filled",
    "star-filled",
    "bookmark-filled",
    "flag-filled",
    "circle-check-filled",
    "shield-filled",
    "crown-filled",
    "coin-filled",
    "gift-filled",
    "sun-filled",
    "moon-filled",
    "cloud-filled",
    "bolt-filled",
    "bell-filled",
    "paw-filled",
    "mood-smile-filled",
    "thumb-down-filled",
    "circle-x-filled",
    "alert-triangle-filled",
    "home-filled",
    "map-pin-filled",
    "camera-filled",
    "palette-filled",
    "briefcase-filled",
    "user-filled",
    "users-group",
    "music",
    "headphones-filled",
    "building-store",
    "building-castle",
    "key-filled",
    "sparkles-filled",
    "book-filled",
    "leaf-filled",
    "compass-filled",
    "device-gamepad-filled",
    "bulb-filled",
    "pin-filled",
    "tag-filled",
    "id-filled",
];

pub const ICON_LABELS: &[&str] = &[
    "contact-visual-icon-0",
    "contact-visual-icon-1",
    "contact-visual-icon-2",
    "contact-visual-icon-3",
    "contact-visual-icon-4",
    "contact-visual-icon-5",
    "contact-visual-icon-6",
    "contact-visual-icon-7",
    "contact-visual-icon-8",
    "contact-visual-icon-9",
    "contact-visual-icon-10",
    "contact-visual-icon-11",
    "contact-visual-icon-12",
    "contact-visual-icon-13",
    "contact-visual-icon-14",
    "contact-visual-icon-15",
    "contact-visual-icon-16",
    "contact-visual-icon-17",
    "contact-visual-icon-18",
    "contact-visual-icon-19",
    "contact-visual-icon-20",
    "contact-visual-icon-21",
    "contact-visual-icon-22",
    "contact-visual-icon-23",
    "contact-visual-icon-24",
    "contact-visual-icon-25",
    "contact-visual-icon-26",
    "contact-visual-icon-27",
    "contact-visual-icon-28",
    "contact-visual-icon-29",
    "contact-visual-icon-30",
    "contact-visual-icon-31",
    "contact-visual-icon-32",
    "contact-visual-icon-33",
    "contact-visual-icon-34",
    "contact-visual-icon-35",
    "contact-visual-icon-36",
    "contact-visual-icon-37",
    "contact-visual-icon-38",
    "contact-visual-icon-39",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PlateSize {
    #[default]
    Inherit,
    Expanded,
    Reduced,
}

/// Where a set's members get a marker in the world (an Aurora addition).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MarkerScope {
    #[default]
    Off,
    /// While they stand on the parcel we stand on.
    Parcel,
    /// While they are in the region we are in.
    Region,
}

/// What a set's marker looks like. Either becomes a plate's round badge
/// when it lands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MarkerShape {
    #[default]
    Circle,
    /// A 45° square.
    Diamond,
}

/// A marker's fade-out distance when the option is first switched on, and
/// what the setting accepts (our avatar to the resident, in metres).
pub const MARKER_FADE_DEFAULT: u16 = 10;
pub const MARKER_FADE_RANGE: std::ops::RangeInclusive<u16> = 1..=128;

/// The marker of one resident: the highest-priority set that has one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub scope: MarkerScope,
    pub shape: MarkerShape,
    pub color: [f32; 4],
    /// The resident's plate border (`Appearance::border`), whichever set
    /// gives it; None: the plates' plain one.
    pub border: Option<[f32; 4]>,
    /// The resident's plate background (`Appearance::background`), likewise.
    pub background: Option<[f32; 4]>,
    /// Into `ICONS`.
    pub icon: Option<usize>,
    /// Metres from our avatar under which the marker is gone.
    pub fade: Option<u16>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Visuals {
    pub border: bool,
    pub background: bool,
    pub nameplate_name: bool,
    pub ui_name: bool,
    pub chat_name: bool,
    pub chat_text: bool,
    pub minimap: bool,
    pub icon: Option<usize>,
    pub replace_friend: bool,
    pub size: PlateSize,
    pub marker: MarkerScope,
    pub marker_shape: MarkerShape,
    pub marker_fade: Option<u16>,
}

impl Visuals {
    pub(super) fn read(v: &Llsd) -> Self {
        Self {
            border: v["border"].as_bool(),
            background: v["background"].as_bool(),
            nameplate_name: v["nameplate_name"].as_bool(),
            ui_name: v["ui_name"].as_bool(),
            chat_name: v["chat_name"].as_bool(),
            chat_text: v["chat_text"].as_bool(),
            minimap: v["minimap"].as_bool(),
            replace_friend: v["replace_friend"].as_bool(),
            icon: ICONS.iter().position(|name| *name == v["icon"].as_str()),
            size: match v["size"].as_str() {
                "expanded" => PlateSize::Expanded,
                "reduced" => PlateSize::Reduced,
                _ => PlateSize::Inherit,
            },
            marker: match v["marker"].as_str() {
                "parcel" => MarkerScope::Parcel,
                "region" => MarkerScope::Region,
                _ => MarkerScope::Off,
            },
            marker_shape: match v["marker_shape"].as_str() {
                "diamond" => MarkerShape::Diamond,
                _ => MarkerShape::Circle,
            },
            marker_fade: match v["marker_fade"].as_f64() {
                fade if fade >= 1.0 => Some((fade as u16).min(*MARKER_FADE_RANGE.end())),
                _ => None,
            },
        }
    }

    pub(super) fn write(&self) -> Llsd {
        let mut v = Llsd::new_map();
        for (k, b) in [
            ("border", self.border),
            ("background", self.background),
            ("nameplate_name", self.nameplate_name),
            ("ui_name", self.ui_name),
            ("chat_name", self.chat_name),
            ("chat_text", self.chat_text),
            ("minimap", self.minimap),
            ("replace_friend", self.replace_friend),
        ] {
            v.insert(k, b);
        }
        if let Some(name) = self.icon.and_then(|i| ICONS.get(i)) {
            v.insert("icon", *name);
        }
        v.insert(
            "size",
            match self.size {
                PlateSize::Inherit => "inherit",
                PlateSize::Expanded => "expanded",
                PlateSize::Reduced => "reduced",
            },
        );
        v.insert(
            "marker",
            match self.marker {
                MarkerScope::Off => "off",
                MarkerScope::Parcel => "parcel",
                MarkerScope::Region => "region",
            },
        );
        v.insert(
            "marker_shape",
            match self.marker_shape {
                MarkerShape::Circle => "circle",
                MarkerShape::Diamond => "diamond",
            },
        );
        if let Some(fade) = self.marker_fade {
            v.insert("marker_fade", f64::from(fade));
        }
        v
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetIcon {
    pub set: String,
    pub icon: usize,
    pub color: [f32; 4],
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Appearance {
    pub border: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub nameplate_name: Option<[f32; 4]>,
    pub ui_name: Option<[f32; 4]>,
    pub chat_name: Option<[f32; 4]>,
    pub chat_text: Option<[f32; 4]>,
    pub minimap: Option<[f32; 4]>,
    pub icons: Vec<SetIcon>,
    pub replace_friend: bool,
    pub size: PlateSize,
    pub marker: Option<Marker>,
}

static EMPTY: std::sync::LazyLock<std::sync::Arc<Appearance>> = std::sync::LazyLock::new(|| std::sync::Arc::new(Appearance::default()));

impl ContactSets {
    pub fn appearance(&self, id: &Uuid) -> &std::sync::Arc<Appearance> {
        self.appearances.get(id).unwrap_or(&EMPTY)
    }

    /// Whether any resident has a marker in the world: with every set's
    /// marker off (the default), the markers look at nobody.
    pub fn has_markers(&self) -> bool {
        self.markers
    }

    pub fn ordered_set_names(&self) -> Vec<String> {
        let mut sets: Vec<_> = self.sets.values().collect();
        sets.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.name.cmp(&b.name)));
        sets.into_iter().map(|s| s.name.clone()).collect()
    }

    pub fn move_priority(&mut self, name: &str, up: bool) {
        let names = self.ordered_set_names();
        let Some(i) = names.iter().position(|n| n == name) else { return };
        let to = if up {
            i.checked_sub(1)
        } else {
            (i + 1 < names.len()).then_some(i + 1)
        };
        if let Some(to) = to {
            self.move_priority_to(name, to);
        }
    }

    /// Put `name` at `index` of the priority order (0 is the highest), the
    /// others keeping theirs: a set dragged in People's list of sets.
    pub fn move_priority_to(&mut self, name: &str, index: usize) {
        let mut names = self.ordered_set_names();
        let Some(from) = names.iter().position(|n| n == name) else { return };
        let moved = names.remove(from);
        let index = index.min(names.len());
        names.insert(index, moved);
        let len = names.len();
        let mut changed = from != index;
        for (i, name) in names.iter().enumerate() {
            if let Some(set) = self.sets.get_mut(name) {
                let priority = (len - i) as i32;
                // the priorities of an older file may tie: written out once
                changed |= set.priority != priority;
                set.priority = priority;
            }
        }
        if changed {
            self.changed();
        }
    }

    /// Where `name` stands in the priority order (0 is the highest), and
    /// how many sets there are.
    pub fn priority_rank(&self, name: &str) -> Option<(usize, usize)> {
        let names = self.ordered_set_names();
        names.iter().position(|n| n == name).map(|i| (i, names.len()))
    }

    pub fn set_visuals(&mut self, name: &str, mut visuals: Visuals) {
        if visuals.icon.is_some_and(|i| i >= ICONS.len()) {
            visuals.icon = None;
        }
        visuals.marker_fade = visuals
            .marker_fade
            .map(|fade| fade.clamp(*MARKER_FADE_RANGE.start(), *MARKER_FADE_RANGE.end()));
        if let Some(set) = self.sets.get_mut(name)
            && set.visuals != visuals
        {
            set.visuals = visuals;
            self.changed();
        }
    }

    pub fn set_stack_icons(&mut self, stack: bool) {
        if self.stack_icons != stack {
            self.stack_icons = stack;
            self.changed();
        }
    }

    /// Rebuilt only when membership/settings change; drawing does no set
    /// sorting or cross-product search per resident per frame.
    pub(super) fn rebuild_appearances(&mut self) {
        let mut resolved: HashMap<Uuid, Appearance> = HashMap::new();
        for name in self.ordered_set_names() {
            let set = &self.sets[&name];
            let v = &set.visuals;
            for id in &set.friends {
                let a = resolved.entry(*id).or_default();
                for (enabled, target) in [
                    (v.border, &mut a.border),
                    (v.background, &mut a.background),
                    (v.nameplate_name, &mut a.nameplate_name),
                    (v.ui_name, &mut a.ui_name),
                    (v.chat_name, &mut a.chat_name),
                    (v.chat_text, &mut a.chat_text),
                    (v.minimap, &mut a.minimap),
                ] {
                    if enabled && target.is_none() {
                        *target = Some(set.color);
                    }
                }
                if a.size == PlateSize::Inherit {
                    a.size = v.size;
                }
                if a.marker.is_none() && v.marker != MarkerScope::Off {
                    a.marker = Some(Marker {
                        scope: v.marker,
                        shape: v.marker_shape,
                        color: set.color,
                        border: None,
                        background: None,
                        icon: v.icon.filter(|i| *i < ICONS.len()),
                        fade: v.marker_fade,
                    });
                }
                a.replace_friend |= v.replace_friend && v.icon.is_some_and(|i| i < ICONS.len());
                if let Some(icon) = v.icon.filter(|i| *i < ICONS.len())
                    && (a.icons.is_empty() || self.stack_icons)
                {
                    a.icons.push(SetIcon {
                        set: name.clone(),
                        icon,
                        color: set.color,
                    });
                }
            }
        }
        for a in resolved.values_mut() {
            if let Some(marker) = &mut a.marker {
                marker.border = a.border;
                marker.background = a.background;
            }
        }
        self.markers = resolved.values().any(|a| a.marker.is_some());
        self.appearances = resolved.into_iter().map(|(id, a)| (id, std::sync::Arc::new(a))).collect();
    }
}
