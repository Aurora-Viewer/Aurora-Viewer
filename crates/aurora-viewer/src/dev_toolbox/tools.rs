//! The tools of the Dev Toolbox and the layout the user arranges them in.
//! An identifier never changes once published (docs/DEV_TOOLBOX.md quotes
//! them): a tool that goes away leaves its identifier unused.

use serde::{Deserialize, Serialize};

/// What pressing a tool does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An overlay or a diagnostic turned on until pressed again; not saved
    /// (off at the next launch, like the former Preferences › Debug page).
    Toggle,
    /// Builds a text shown in "Send a diagnostic", to copy or save.
    Copy,
    /// A view kept up to date in "Send a diagnostic".
    Live,
    /// Opens a window (or a short menu of actions).
    Window,
}

#[derive(Debug, Clone, Copy)]
pub struct Tool {
    pub id: &'static str,
    pub kind: Kind,
    /// Tabler icon (ui/icons.rs).
    pub icon: &'static str,
    /// The group it goes back to when the layout is reset.
    pub group: Group,
}

/// The built-in groups, in their default order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Favorites,
    Reports,
    Live,
    Rendering,
    Objects,
    Network,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Favorites,
        Group::Reports,
        Group::Live,
        Group::Rendering,
        Group::Objects,
        Group::Network,
    ];

    /// Its key in the saved layout.
    pub fn key(self) -> &'static str {
        match self {
            Group::Favorites => "favorites",
            Group::Reports => "reports",
            Group::Live => "live",
            Group::Rendering => "rendering",
            Group::Objects => "objects",
            Group::Network => "network",
        }
    }

    pub fn from_key(key: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.key() == key)
    }
}

use Group as G;
use Kind as K;

const fn t(id: &'static str, kind: Kind, icon: &'static str, group: Group) -> Tool {
    Tool { id, kind, icon, group }
}

/// Every tool, in catalogue order.
pub const TOOLS: &[Tool] = &[
    t("J1", K::Copy, "file-text-filled", G::Reports),
    t("J2", K::Copy, "clipboard-text-filled", G::Reports),
    t("J3", K::Copy, "alert-triangle-filled", G::Reports),
    t("J4", K::Copy, "cpu", G::Reports),
    t("J5", K::Copy, "file-export", G::Reports),
    t("J6", K::Window, "folder-open-filled", G::Reports),
    t("J7", K::Window, "terminal-2", G::Reports),
    t("J8", K::Copy, "bone-filled", G::Reports),
    t("L1", K::Live, "crosshair", G::Live),
    t("L2", K::Live, "stack-2-filled", G::Live),
    t("L3", K::Live, "alert-triangle-filled", G::Live),
    t("N3", K::Live, "download-filled", G::Live),
    t("R1", K::Toggle, "eye-filled", G::Rendering),
    t("R2", K::Toggle, "eye-filled", G::Rendering),
    t("R3", K::Toggle, "layout-grid-filled", G::Rendering),
    t("R4", K::Toggle, "vector", G::Rendering),
    t("R5", K::Toggle, "sun-filled", G::Rendering),
    t("R6", K::Toggle, "stack-filled", G::Rendering),
    t("R7", K::Toggle, "aperture", G::Rendering),
    t("R8", K::Toggle, "brightness-filled", G::Rendering),
    t("R9", K::Toggle, "bulb-filled", G::Rendering),
    t("R10", K::Toggle, "sphere", G::Rendering),
    t("R11", K::Toggle, "arrows-sort", G::Rendering),
    t("R12", K::Toggle, "arrows-sort", G::Rendering),
    t("O1", K::Toggle, "box", G::Objects),
    t("O2", K::Toggle, "layers-intersect", G::Objects),
    t("O3", K::Toggle, "box-model-2", G::Objects),
    t("O4", K::Toggle, "snowflake", G::Objects),
    t("O5", K::Toggle, "bone-filled", G::Objects),
    t("O6", K::Toggle, "chart-dots-filled", G::Objects),
    t("O7", K::Window, "search-filled", G::Objects),
    t("O8", K::Window, "medical-cross-filled", G::Objects),
    t("O9", K::Toggle, "shape", G::Objects),
    t("N1", K::Window, "chart-line", G::Network),
    t("N2", K::Copy, "world-filled", G::Network),
    t("P1", K::Window, "gauge-filled", G::Network),
    t("P2", K::Window, "heart-rate-monitor", G::Network),
    t("P3", K::Toggle, "hourglass-filled", G::Network),
    t("P4", K::Toggle, "player-pause-filled", G::Network),
    t("P5", K::Toggle, "route", G::Network),
    t("P6", K::Toggle, "clock-filled", G::Network),
    t("M1", K::Toggle, "movie", G::Network),
];

/// The favourites of a new layout.
const DEFAULT_FAVORITES: &[&str] = &["J2", "L1", "R3", "P1"];

pub fn find(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.id == id)
}

/// The tool's label, in the interface language.
pub fn label(id: &str) -> &'static str {
    use aurora_i18n::tr;
    match id {
        "J1" => tr!("dev-toolbox-j1"),
        "J2" => tr!("dev-toolbox-j2"),
        "J3" => tr!("dev-toolbox-j3"),
        "J4" => tr!("dev-toolbox-j4"),
        "J5" => tr!("dev-toolbox-j5"),
        "J6" => tr!("dev-toolbox-j6"),
        "J7" => tr!("dev-toolbox-j7"),
        "J8" => tr!("dev-toolbox-j8"),
        "L1" => tr!("dev-toolbox-l1"),
        "L2" => tr!("dev-toolbox-l2"),
        "L3" => tr!("dev-toolbox-l3"),
        "N3" => tr!("dev-toolbox-n3"),
        "R1" => tr!("dev-toolbox-r1"),
        "R2" => tr!("dev-toolbox-r2"),
        "R3" => tr!("dev-toolbox-r3"),
        "R4" => tr!("dev-toolbox-r4"),
        "R5" => tr!("dev-toolbox-r5"),
        "R6" => tr!("dev-toolbox-r6"),
        "R7" => tr!("dev-toolbox-r7"),
        "R8" => tr!("dev-toolbox-r8"),
        "R9" => tr!("dev-toolbox-r9"),
        "R10" => tr!("dev-toolbox-r10"),
        "R11" => tr!("dev-toolbox-r11"),
        "R12" => tr!("dev-toolbox-r12"),
        "O1" => tr!("dev-toolbox-o1"),
        "O2" => tr!("dev-toolbox-o2"),
        "O3" => tr!("dev-toolbox-o3"),
        "O4" => tr!("dev-toolbox-o4"),
        "O5" => tr!("dev-toolbox-o5"),
        "O6" => tr!("dev-toolbox-o6"),
        "O7" => tr!("dev-toolbox-o7"),
        "O8" => tr!("dev-toolbox-o8"),
        "O9" => tr!("dev-toolbox-o9"),
        "N1" => tr!("dev-toolbox-n1"),
        "N2" => tr!("dev-toolbox-n2"),
        "P1" => tr!("dev-toolbox-p1"),
        "P2" => tr!("dev-toolbox-p2"),
        "P3" => tr!("dev-toolbox-p3"),
        "P4" => tr!("dev-toolbox-p4"),
        "P5" => tr!("dev-toolbox-p5"),
        "P6" => tr!("dev-toolbox-p6"),
        "M1" => tr!("dev-toolbox-m1"),
        _ => "?",
    }
}

/// What the second line of a button says when the tool has nothing live.
pub fn kind_label(kind: Kind) -> &'static str {
    use aurora_i18n::tr;
    match kind {
        Kind::Toggle => tr!("dev-toolbox-kind-toggle"),
        Kind::Copy => tr!("dev-toolbox-kind-copy"),
        Kind::Live => tr!("dev-toolbox-kind-live"),
        Kind::Window => tr!("dev-toolbox-kind-window"),
    }
}

/// A built-in group's title and hint.
pub fn group_title(g: Group) -> (&'static str, &'static str) {
    use aurora_i18n::tr;
    match g {
        Group::Favorites => (tr!("dev-toolbox-group-favorites"), tr!("dev-toolbox-group-favorites-hint")),
        Group::Reports => (tr!("dev-toolbox-group-reports"), tr!("dev-toolbox-group-reports-hint")),
        Group::Live => (tr!("dev-toolbox-group-live"), tr!("dev-toolbox-group-live-hint")),
        Group::Rendering => (tr!("dev-toolbox-group-rendering"), tr!("dev-toolbox-group-rendering-hint")),
        Group::Objects => (tr!("dev-toolbox-group-objects"), tr!("dev-toolbox-group-objects-hint")),
        Group::Network => (tr!("dev-toolbox-group-network"), ""),
    }
}

/// Whether a tool matches the search field: its identifier or its label.
pub fn matches(id: &str, label: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || id.to_lowercase().contains(&q) || label.to_lowercase().contains(&q)
}

/// One group of the layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupLayout {
    /// A built-in group's key ([`Group::key`]), or `custom-<n>`.
    pub key: String,
    /// A custom group's title (empty for a built-in one).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub tools: Vec<String>,
}

/// The arrangement the user chose (Customize), saved in the settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Layout {
    pub groups: Vec<GroupLayout>,
    /// Every tool that existed when it was saved: one missing from the
    /// groups was removed by the user and stays out, one not listed here is
    /// new in this version and joins its group.
    #[serde(default)]
    pub known: Vec<String>,
}

impl Layout {
    pub fn default_layout() -> Layout {
        let groups = Group::ALL
            .into_iter()
            .map(|g| GroupLayout {
                key: g.key().into(),
                title: String::new(),
                tools: if g == Group::Favorites {
                    DEFAULT_FAVORITES.iter().map(|s| s.to_string()).collect()
                } else {
                    TOOLS.iter().filter(|t| t.group == g).map(|t| t.id.to_string()).collect()
                },
            })
            .collect();
        Layout {
            groups,
            known: TOOLS.iter().map(|t| t.id.to_string()).collect(),
        }
    }

    /// A saved layout made valid for this version: unknown tools dropped,
    /// a tool at most once outside the favourites, Favorites first, the
    /// tools new since it was saved added to their group.
    pub fn normalized(saved: Option<&Layout>) -> Layout {
        let Some(saved) = saved.filter(|l| !l.groups.is_empty()) else {
            return Layout::default_layout();
        };
        let mut layout = saved.clone();
        let mut seen = std::collections::HashSet::new();
        for g in &mut layout.groups {
            let favorites = g.key == Group::Favorites.key();
            let mut here = std::collections::HashSet::new();
            g.tools
                .retain(|id| find(id).is_some() && here.insert(id.clone()) && (favorites || seen.insert(id.clone())));
        }
        if let Some(i) = layout.groups.iter().position(|g| g.key == Group::Favorites.key()) {
            let favorites = layout.groups.remove(i);
            layout.groups.insert(0, favorites);
        }
        for tool in TOOLS.iter().filter(|t| !saved.known.iter().any(|k| k == t.id)) {
            if seen.contains(tool.id) {
                continue;
            }
            let key = tool.group.key();
            match layout.groups.iter_mut().find(|g| g.key == key) {
                Some(g) => g.tools.push(tool.id.into()),
                None => layout.groups.push(GroupLayout {
                    key: key.into(),
                    title: String::new(),
                    tools: vec![tool.id.into()],
                }),
            }
        }
        layout.known = TOOLS.iter().map(|t| t.id.to_string()).collect();
        layout
    }

    /// The tools in no group outside the favourites (removed by the user):
    /// "Add a tool" offers them.
    pub fn removed(&self) -> Vec<&'static str> {
        TOOLS
            .iter()
            .map(|t| t.id)
            .filter(|id| {
                !self
                    .groups
                    .iter()
                    .any(|g| g.key != Group::Favorites.key() && g.tools.iter().any(|t| t == id))
            })
            .collect()
    }

    /// Move the tool at `from` (group, rank) to `to` (group, rank before the
    /// move). A tool dropped in Favorites is copied there, not moved; one
    /// dragged out of Favorites leaves it.
    pub fn move_tool(&mut self, from: (usize, usize), to: (usize, usize)) {
        let Some(id) = self.groups.get(from.0).and_then(|g| g.tools.get(from.1)).cloned() else {
            return;
        };
        if to.0 >= self.groups.len() {
            return;
        }
        let favorites = |l: &Layout, g: usize| l.groups[g].key == Group::Favorites.key();
        let into_favorites = favorites(self, to.0) && !favorites(self, from.0);
        if into_favorites {
            if !self.groups[to.0].tools.contains(&id) {
                let rank = to.1.min(self.groups[to.0].tools.len());
                self.groups[to.0].tools.insert(rank, id);
            }
            return;
        }
        if favorites(self, from.0) && !favorites(self, to.0) {
            // out of Favorites: the tool is still in its own group
            self.groups[from.0].tools.remove(from.1);
            return;
        }
        self.groups[from.0].tools.remove(from.1);
        let mut rank = to.1;
        if from.0 == to.0 && from.1 < to.1 {
            rank -= 1;
        }
        let rank = rank.min(self.groups[to.0].tools.len());
        self.groups[to.0].tools.insert(rank, id);
    }

    /// Remove a tool from a group (right-click › Remove).
    pub fn remove_tool(&mut self, group: usize, rank: usize) {
        if let Some(g) = self.groups.get_mut(group)
            && rank < g.tools.len()
        {
            g.tools.remove(rank);
        }
    }

    /// Put a removed tool back, at the end of `group`.
    pub fn add_tool(&mut self, group: usize, id: &str) {
        if find(id).is_none() {
            return;
        }
        let favorites = self.groups.get(group).is_some_and(|g| g.key == Group::Favorites.key());
        if !favorites {
            for g in self.groups.iter_mut().filter(|g| g.key != Group::Favorites.key()) {
                g.tools.retain(|t| t != id);
            }
        }
        if let Some(g) = self.groups.get_mut(group)
            && !g.tools.iter().any(|t| t == id)
        {
            g.tools.push(id.into());
        }
    }

    /// Pin or unpin a tool in Favorites (right-click › Pin to Favorites).
    pub fn toggle_favorite(&mut self, id: &str) {
        let Some(g) = self.groups.iter_mut().find(|g| g.key == Group::Favorites.key()) else {
            return;
        };
        if let Some(i) = g.tools.iter().position(|t| t == id) {
            g.tools.remove(i);
        } else {
            g.tools.push(id.into());
        }
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.groups
            .iter()
            .any(|g| g.key == Group::Favorites.key() && g.tools.iter().any(|t| t == id))
    }

    /// Move a whole group (by its title); Favorites stays first.
    pub fn move_group(&mut self, from: usize, to: usize) {
        if from == 0 || from >= self.groups.len() {
            return;
        }
        let g = self.groups.remove(from);
        let to = if from < to { to - 1 } else { to }.clamp(1, self.groups.len());
        self.groups.insert(to, g);
    }

    /// A new empty group at the end ("New group").
    pub fn add_group(&mut self, title: &str) {
        let n = (1..)
            .find(|n| !self.groups.iter().any(|g| g.key == format!("custom-{n}")))
            .unwrap_or(1);
        self.groups.push(GroupLayout {
            key: format!("custom-{n}"),
            title: title.into(),
            tools: Vec::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(l: &Layout, g: usize) -> Vec<&str> {
        l.groups[g].tools.iter().map(String::as_str).collect()
    }

    #[test]
    fn identifiers_are_unique_and_labelled() {
        let mut seen = std::collections::HashSet::new();
        for t in TOOLS {
            assert!(seen.insert(t.id), "{} twice", t.id);
            assert_ne!(label(t.id), "?", "{} has no label", t.id);
        }
    }

    #[test]
    fn default_layout_puts_every_tool_in_a_group() {
        let l = Layout::default_layout();
        assert_eq!(l.groups[0].key, "favorites");
        assert_eq!(ids(&l, 0), ["J2", "L1", "R3", "P1"]);
        assert!(l.removed().is_empty());
        assert_eq!(Layout::normalized(None), l);
    }

    #[test]
    fn new_tools_join_their_group_and_removed_ones_stay_out() {
        let mut saved = Layout::default_layout();
        // saved before R12 existed, with R4 removed by the user
        saved.known.retain(|k| k != "R12");
        for g in &mut saved.groups {
            g.tools.retain(|t| t != "R12" && t != "R4" && t != "XX");
        }
        saved.groups[1].tools.push("XX".into());
        let l = Layout::normalized(Some(&saved));
        let rendering = l.groups.iter().find(|g| g.key == "rendering").unwrap();
        assert!(rendering.tools.iter().any(|t| t == "R12"));
        assert!(!rendering.tools.iter().any(|t| t == "R4"));
        assert_eq!(l.removed(), ["R4"]);
        assert!(!l.groups[1].tools.iter().any(|t| t == "XX"));
    }

    #[test]
    fn moving_within_and_between_groups() {
        let mut l = Layout::default_layout();
        // J1 after J3 in Reports
        l.move_tool((1, 0), (1, 3));
        assert_eq!(&ids(&l, 1)[..3], ["J2", "J3", "J1"]);
        // R1 to Favorites: copied
        let r = l.groups.iter().position(|g| g.key == "rendering").unwrap();
        l.move_tool((r, 0), (0, 0));
        assert_eq!(ids(&l, 0)[0], "R1");
        assert_eq!(ids(&l, r)[0], "R1");
        // out of Favorites again: unpinned only
        l.move_tool((0, 0), (r, 5));
        assert!(!l.is_favorite("R1"));
        assert_eq!(ids(&l, r)[0], "R1");
        // R1 into Objects: moved
        let o = l.groups.iter().position(|g| g.key == "objects").unwrap();
        l.move_tool((r, 0), (o, 0));
        assert_eq!(ids(&l, o)[0], "R1");
        assert_ne!(ids(&l, r)[0], "R1");
    }

    #[test]
    fn groups_move_but_favorites_stay_first() {
        let mut l = Layout::default_layout();
        l.move_group(5, 1);
        assert_eq!(l.groups[1].key, "network");
        l.move_group(1, 0);
        assert_eq!(l.groups[0].key, "favorites");
        l.add_group("Mine");
        assert_eq!(l.groups.last().map(|g| g.key.as_str()), Some("custom-1"));
    }

    #[test]
    fn search_matches_identifier_or_label() {
        assert!(matches("J2", "Debug to clipboard", "j2"));
        assert!(matches("J2", "Debug to clipboard", "CLIP"));
        assert!(!matches("J2", "Debug to clipboard", "wire"));
        assert!(matches("J2", "Debug to clipboard", "  "));
    }
}
