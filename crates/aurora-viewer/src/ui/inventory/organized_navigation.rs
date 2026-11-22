//! Human-readable addresses scoped to the displayed inventory namespace.
use super::*;
use aurora_i18n::tr;

#[derive(Default)]
pub(super) struct Navigation {
    pub(super) focus: bool,
    editing: Option<Editor>,
    stamp: Option<(Uuid, u64, u64, bool)>,
    avatar: String,
    paths: HashMap<Location, String>,
    duplicates: HashMap<Location, HashSet<String>>,
}

struct Editor {
    text: String,
    pending: Option<Uuid>,
    error: String,
    choices: Vec<String>,
}
impl Navigation {
    pub(super) fn clear_cache(&mut self) {
        self.paths.clear();
        self.duplicates.clear();
    }
    pub(super) fn close(&mut self) {
        self.focus = false;
        self.editing = None;
    }
    pub(super) fn active(&self) -> bool {
        self.focus || self.editing.is_some()
    }

    pub(super) fn address(&mut self, inv: &Inventory, layout: &Preferences, place: Location, raw: bool, avatar: &str) -> String {
        let stamp = (inv.root, inv.generation, layout.revision(), raw);
        if self.stamp != Some(stamp) || self.avatar != avatar || self.paths.len() > 4096 || self.duplicates.len() > 4096 {
            self.clear_cache();
            self.stamp = Some(stamp);
            self.avatar.clear();
            self.avatar.push_str(avatar);
        }
        if let Some(address) = self.paths.get(&place) {
            return address.clone();
        }
        let chain = path(inv, layout, place, raw);
        let mut parts = Vec::with_capacity(chain.len());
        for (index, &child) in chain.iter().enumerate() {
            let name = place_name(inv, layout, child, raw, avatar);
            let mut segment = encode(&name);
            if let Some(&up) = index.checked_sub(1).map(|i| &chain[i]) {
                let duplicates = self.duplicates.entry(up).or_insert_with(|| {
                    let mut unique = HashSet::new();
                    let mut repeated = HashSet::new();
                    for sibling in folders(inv, layout, up, raw) {
                        let label = place_name(inv, layout, sibling, raw, avatar).to_lowercase();
                        let alias_collision =
                            !raw && up == Location::Home && sibling != Location::MyFolders && personal_address_name(&label);
                        if !unique.insert(label.clone()) || alias_collision {
                            repeated.insert(label);
                        }
                    }
                    repeated
                });
                if duplicates.contains(&name.to_lowercase()) {
                    segment.push('@');
                    segment.push_str(&qualifier(child));
                }
            }
            parts.push(segment);
        }
        let address = parts.join("\\");
        self.paths.insert(place, address.clone());
        address
    }
}

fn encode(s: &str) -> String {
    if s == "." {
        return "%2E".into();
    }
    if s == ".." {
        return "%2E%2E".into();
    }
    s.replace('%', "%25").replace('\\', "%5C").replace('/', "%2F").replace('@', "%40")
}

fn decode(s: &str) -> String {
    let mut output = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('%') {
        output.push_str(&rest[..pos]);
        rest = &rest[pos + 1..];
        let ch = rest.get(..2).and_then(|hex| match hex.to_ascii_uppercase().as_str() {
            "25" => Some('%'),
            "5C" => Some('\\'),
            "2F" => Some('/'),
            "40" => Some('@'),
            "2E" => Some('.'),
            _ => None,
        });
        if let Some(ch) = ch {
            output.push(ch);
            rest = &rest[2..];
        } else {
            output.push('%');
        }
    }
    output.push_str(rest);
    output
}

fn qualifier(place: Location) -> String {
    match place {
        Location::Folder(id) => format!("f:{id}"),
        Location::Custom(id) => format!("v:{id}"),
        Location::Group(group) => format!("g:{group:?}"),
        Location::Library => "library".into(),
        Location::FullTree => "original".into(),
        Location::Home => "home".into(),
        Location::MyFolders => "personal".into(),
    }
}

// Only a valid generated identity suffix is special. Users may type ordinary
// names such as "Store @ Home" directly, as well as paste their escaped form.
fn split_segment(s: &str) -> (&str, Option<&str>) {
    if let Some((label, tag)) = s.rsplit_once('@') {
        let known = match tag.split_once(':') {
            Some(("f" | "v", uuid)) => Uuid::parse_str(uuid).is_ok(),
            Some(("g", name)) => Group::ALL.iter().any(|g| format!("{g:?}") == name),
            None => matches!(tag, "home" | "personal" | "library" | "original"),
            _ => false,
        };
        if known {
            return (label, Some(tag));
        }
    }
    (s, None)
}
fn folders(inv: &Inventory, layout: &Preferences, place: Location, raw: bool) -> Vec<Location> {
    if raw {
        return canonical_children(inv, place)
            .into_iter()
            .filter_map(|r| if let Row::Place(p) = r { Some(p) } else { None })
            .collect();
    }
    if place == Location::Home {
        // Accept the same destinations as the dashboard, including nested pin shortcuts.
        let mut out = vec![Location::MyFolders];
        out.extend(layout.home_roots(inv));
        out.extend(layout.home_children(inv));
        out.extend(layout.pins(inv));
        let mut seen = HashSet::new();
        out.retain(|place| seen.insert(*place));
        return out;
    }
    let mut out = layout.virtual_children(inv, place);
    out.extend(
        layout
            .children(inv, place)
            .into_iter()
            .filter(|id| *id != inv.lib_root && inv.folders.contains_key(id))
            .map(Location::Folder),
    );
    out
}

// A copied address may outlive both a release and a change of viewer language.
fn personal_address_name(folded: &str) -> bool {
    use std::sync::OnceLock;
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            aurora_i18n::available_languages()
                .flat_map(|(language, _)| {
                    ["explorer-home-my-folders", "explorer-general-inventory"].map(|key| aurora_i18n::text_in(language, key).to_lowercase())
                })
                .collect()
        })
        .contains(folded)
}

enum Lookup {
    Open(Location, bool),
    Load(Uuid),
    Missing(String),
    Ambiguous(String, Vec<String>),
}

fn resolve(inv: &Inventory, layout: &Preferences, from: Location, raw: bool, avatar: &str, text: &str) -> Lookup {
    let mut raw = raw;
    let mut root = if raw { Location::FullTree } else { Location::Home };
    let absolute = text.starts_with(['\\', '/']);
    let parts: Vec<_> = text.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return if absolute {
            Lookup::Open(root, raw)
        } else {
            Lookup::Missing(String::new())
        };
    }
    let first = decode(parts[0]).to_lowercase();
    let mut start = 0;
    let mut place = if absolute { root } else { from };
    let home = place_name(inv, layout, Location::Home, false, avatar);
    let original = place_name(inv, layout, Location::FullTree, true, avatar);
    if first == home.to_lowercase() || first == original.to_lowercase() {
        // A raw path keeps its own root even if the avatar and translated
        // original-tree labels happen to coincide.
        raw = first == original.to_lowercase() && (raw || first != home.to_lowercase());
        root = if raw { Location::FullTree } else { Location::Home };
        place = root;
        start = 1;
    }
    for (index, &part) in parts.iter().enumerate().skip(start) {
        if part == "." {
            continue;
        }
        if part == ".." {
            place = parent(inv, layout, place, raw).unwrap_or(root);
            continue;
        }
        let (label, tag) = split_segment(part);
        let label = decode(label);
        let folded = label.to_lowercase();
        let personal_alias = !raw && place == Location::Home && personal_address_name(&folded);
        let named = |loc: &Location| {
            (place_name(inv, layout, *loc, raw, avatar).to_lowercase() == folded || (*loc == Location::MyFolders && personal_alias))
                && tag.is_none_or(|t| qualifier(*loc) == t)
        };
        let mut candidates: Vec<_> = folders(inv, layout, place, raw).into_iter().filter(named).collect();
        if !raw
            && place == Location::Home
            && candidates
                .iter()
                .all(|loc| layout.location_parent(inv, *loc) != Some(Location::Home))
        {
            // Old personal-root paths remain usable. If a newly pinned shortcut
            // shares the old folder's name, neither shortcut may silently win.
            for old in folders(inv, layout, Location::MyFolders, false).into_iter().filter(named) {
                if !candidates.contains(&old) {
                    candidates.push(old);
                }
            }
        }
        if tag.is_none() && candidates.len() > 1 {
            let mut choices: Vec<_> = candidates
                .iter()
                .map(|loc| {
                    let mut choice: Vec<_> = parts.iter().map(|s| s.to_string()).collect();
                    choice[index] = format!("{}@{}", encode(&place_name(inv, layout, *loc, raw, avatar)), qualifier(*loc));
                    format!("{}{}", if absolute { "\\" } else { "" }, choice.join("\\"))
                })
                .collect();
            choices.sort();
            return Lookup::Ambiguous(label, choices);
        }
        // A qualified address already identifies a specific folder. Bare
        // names need complete siblings before choosing between duplicates.
        let physical = match place {
            Location::Home | Location::MyFolders | Location::FullTree | Location::Group(_) => Some(inv.root),
            Location::Library => Some(inv.lib_root),
            Location::Folder(id) => Some(id),
            Location::Custom(_) => None,
        };
        if tag.is_none() || candidates.is_empty() {
            let backing = (!raw && place == Location::Home).then(|| model::home_folder(inv, layout)).flatten();
            if let Some(id) = physical
                .into_iter()
                .chain(backing)
                .find(|id| inv.folders.get(id).is_some_and(|f| f.state != FetchState::Fetched))
            {
                return Lookup::Load(id);
            }
        }
        match candidates.as_slice() {
            [next] => place = *next,
            _ => return Lookup::Missing(label),
        }
    }
    Lookup::Open(place, raw)
}

pub(super) fn search_location(inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, row: Row, avatar: &str) -> Option<String> {
    if state.here.query.trim().is_empty() {
        return None;
    }
    let raw = state.here.raw;
    Some(
        state
            .navigation
            .address(inv, layout, containing(inv, layout, row, raw)?, raw, avatar),
    )
}

pub(super) fn containing(inv: &Inventory, layout: &Preferences, row: Row, raw: bool) -> Option<Location> {
    Some(match row {
        Row::Place(place) => parent(inv, layout, place, raw)?,
        Row::Item(id) if raw => {
            let folder = inv.items.get(&id)?.parent;
            if folder == inv.root {
                Location::FullTree
            } else {
                Location::Folder(folder)
            }
        }
        Row::Item(id) => layout.parent(inv, id),
    })
}

pub(super) fn address_field(ui: &mut Ui, inv: &mut Inventory, layout: &Preferences, state: &mut ExplorerState, avatar: &str) -> bool {
    let focus = std::mem::take(&mut state.navigation.focus);
    if focus && state.navigation.editing.is_none() {
        let text = state.navigation.address(inv, layout, state.here.location, state.here.raw, avatar);
        state.navigation.editing = Some(Editor {
            text,
            pending: None,
            error: String::new(),
            choices: Vec::new(),
        });
    }
    let Some(editor) = &mut state.navigation.editing else {
        return false;
    };
    let mut output = egui::TextEdit::singleline(&mut editor.text)
        .id_salt("inventory-address")
        .align(Align2::LEFT_CENTER)
        .min_size(vec2(1.0, ui.available_height()))
        .margin(egui::Margin::symmetric(6, 2))
        .desired_width(ui.available_width())
        .show(ui);
    if focus {
        output.response.request_focus();
        let range = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(editor.text.chars().count()));
        output.state.cursor.set_char_range(Some(range));
        output.state.store(ui.ctx(), output.response.id);
    }
    if output.response.changed() {
        editor.error.clear();
        editor.choices.clear();
        editor.pending = None;
    }
    let enter = output.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let escape = (output.response.has_focus() || output.response.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape));
    if escape || (output.response.lost_focus() && !enter && editor.error.is_empty() && !focus) {
        state.navigation.close();
        return true;
    }
    let completed = editor
        .pending
        .is_some_and(|id| inv.folders.get(&id).is_none_or(|f| f.state == FetchState::Fetched));
    if editor
        .pending
        .is_some_and(|id| inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Failed))
    {
        editor.pending = None;
        editor.error = tr!("explorer-address-fetch-failed").into();
    }
    if enter || completed {
        editor.pending = None;
        editor.choices.clear();
        match resolve(inv, layout, state.here.location, state.here.raw, avatar, &editor.text) {
            Lookup::Open(place, raw) => {
                state.navigation.close();
                state.visit(place, raw);
            }
            Lookup::Missing(name) => {
                editor.error = tr!("explorer-address-missing", name = name);
                output.response.request_focus();
            }
            Lookup::Ambiguous(name, choices) => {
                editor.error = tr!("explorer-address-ambiguous", name = name);
                editor.choices = choices;
                output.response.request_focus();
            }
            Lookup::Load(id) => {
                inv.request(id);
                editor.pending = Some(id);
                editor.error = tr!("explorer-address-loading").into();
                output.response.request_focus();
            }
        }
    }
    true
}

pub(super) fn feedback(ui: &mut Ui, state: &mut ExplorerState) {
    let Some(editor) = &mut state.navigation.editing else {
        return;
    };
    if editor.error.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(&editor.error);
        if !editor.choices.is_empty() {
            ui.menu_button(tr!("explorer-address-choose"), |ui| {
                ui.set_max_width(ui.ctx().content_rect().width().min(560.0));
                for choice in &editor.choices {
                    if ui.add(egui::Button::new(choice).truncate()).on_hover_text(choice).clicked() {
                        editor.text.clone_from(choice);
                        editor.error.clear();
                        state.navigation.focus = true;
                        ui.close();
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, id};
    use super::*;
    fn opened(lookup: Lookup) -> (Location, bool) {
        match lookup {
            Lookup::Open(loc, raw) => (loc, raw),
            _ => panic!("expected valid displayed address"),
        }
    }

    #[test]
    fn renamed_general_inventory_accepts_old_translated_addresses_and_qualifies_literal_home_aliases() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        for (language, _) in aurora_i18n::available_languages() {
            let old = aurora_i18n::text_in(language, "explorer-home-my-folders");
            assert_eq!(
                opened(resolve(
                    &inv,
                    &layout,
                    Location::Home,
                    false,
                    "Tester",
                    &format!("Tester\\{old}\\Workshop")
                )),
                (Location::Folder(id(3)), false)
            );
        }
        // A real Home folder with the old display name must round-trip too.
        layout.set_home_folder(id(3));
        let child = id(4);
        inv.folders.get_mut(&child).unwrap().info.parent = id(3);
        inv.folders.get_mut(&child).unwrap().info.name = "My Folders".into();
        inv.folders.get_mut(&id(3)).unwrap().children.push(child);
        let mut nav = Navigation::default();
        assert!(matches!(
            resolve(&inv, &layout, Location::Home, false, "Tester", "Tester\\My Folders"),
            Lookup::Ambiguous(..)
        ));
        let display = nav.address(&inv, &layout, Location::Folder(child), false, "Tester");
        assert!(display.contains("@f:"));
        assert_eq!(
            opened(resolve(&inv, &layout, Location::Home, false, "Tester", &display)),
            (Location::Folder(child), false)
        );
        inv.folders.get_mut(&id(3)).unwrap().state = FetchState::Unknown;
        assert!(matches!(resolve(&inv, &layout, Location::Home, false, "Tester", r"Tester\Unfetched"), Lookup::Load(x) if x == id(3)));
    }

    #[test]
    fn organized_addresses_round_trip_duplicates_delimiters_and_unicode() {
        let mut inv = fixture();
        let layout = Preferences::default();
        let mut nav = Navigation::default();
        let up = id(3);
        let literal = "衣装/hat\\right@%25";
        for value in [40, 41] {
            let uuid = id(value);
            let mut f = inv.folders[&up].clone();
            f.info.id = uuid;
            f.info.parent = up;
            f.info.name = literal.into();
            f.items.clear();
            f.children.clear();
            inv.folders.insert(uuid, f);
            inv.folders.get_mut(&up).unwrap().children.push(uuid);
        }
        let display = nav.address(&inv, &layout, Location::Folder(id(40)), false, "Tester");
        assert!(display.contains("衣装%2Fhat%5Cright%40%2525@f:"));
        assert_eq!(
            opened(resolve(&inv, &layout, Location::Home, false, "Tester", &display)),
            (Location::Folder(id(40)), false)
        );
        let bare = display.split('@').next().unwrap();
        let Lookup::Ambiguous(_, choices) = resolve(&inv, &layout, Location::Home, false, "Tester", bare) else {
            panic!("duplicate names must not choose an arbitrary folder")
        };
        assert_eq!(choices.len(), 2);
        for (n, choice) in [40, 41].into_iter().zip(choices) {
            assert_eq!(
                opened(resolve(&inv, &layout, Location::Home, false, "Tester", &choice)).0,
                Location::Folder(id(n))
            );
        }
        inv.folders.get_mut(&id(41)).unwrap().info.name = "Store @ Home".into();
        assert_eq!(
            opened(resolve(&inv, &layout, Location::Folder(up), false, "Tester", "Store @ Home")).0,
            Location::Folder(id(41))
        );
        assert_eq!(
            opened(resolve(&inv, &layout, Location::Folder(up), false, "Tester", "Store %40 Home")).0,
            Location::Folder(id(41))
        );
        assert!(matches!(
            resolve(&inv, &layout, Location::Home, false, "Tester", "Tester/missing"),
            Lookup::Missing(_)
        ));
    }

    #[test]
    fn organized_home_and_real_my_folders_round_trip_even_when_both_are_pinned() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let archive = Location::Folder(id(4));
        let mut nav = Navigation::default();
        assert_eq!(
            opened(resolve(&inv, &layout, Location::Home, false, "Tester", "Tester\\Archive")),
            (archive, false)
        );
        inv.folders.get_mut(&id(4)).unwrap().info.name = layout.location_label(&inv, Location::MyFolders).to_owned();
        // An existing real folder can have the same label as the built-in tile.
        assert!(layout.pin_location(&inv, archive));
        let Lookup::Ambiguous(_, choices) = resolve(&inv, &layout, Location::Home, false, "Tester", "Tester\\General Inventory") else {
            panic!("ambiguous pin must not shadow the built-in destination");
        };
        assert_eq!(choices.len(), 2);
        for place in [Location::MyFolders, archive] {
            let address = nav.address(&inv, &layout, place, false, "Tester");
            assert_eq!(
                opened(resolve(&inv, &layout, Location::Home, false, "Tester", &address)),
                (place, false)
            );
        }
    }

    #[test]
    fn organized_old_address_and_new_nested_pin_with_same_name_require_a_choice() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let nested = id(17);
        let mut folder = inv.folders[&id(4)].clone();
        folder.info.id = nested;
        folder.info.parent = id(3);
        folder.children.clear();
        folder.items.clear();
        inv.folders.get_mut(&id(3)).unwrap().children.push(nested);
        inv.folders.insert(nested, folder);
        assert!(layout.pin_location(&inv, Location::Folder(nested)));
        let Lookup::Ambiguous(_, choices) = resolve(&inv, &layout, Location::Home, false, "Tester", "Tester/Archive") else {
            panic!("old personal path must not silently select a newer shortcut");
        };
        let selected: HashSet<_> = choices
            .iter()
            .map(|choice| opened(resolve(&inv, &layout, Location::Home, false, "Tester", choice)).0)
            .collect();
        assert_eq!(selected, HashSet::from([Location::Folder(id(4)), Location::Folder(nested)]));
    }

    #[test]
    fn organized_paths_use_current_namespace_and_wait_for_folder_contents() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let workshop = Location::Folder(id(3));
        let archive = Location::Folder(id(4));
        assert!(layout.place(&inv, id(3), archive));
        let mut nav = Navigation::default();
        let display = nav.address(&inv, &layout, workshop, false, "Tester");
        assert_eq!(display, "Tester\\General Inventory\\Archive\\Workshop");
        assert_eq!(
            opened(resolve(&inv, &layout, Location::Home, false, "Tester", &display)),
            (workshop, false)
        );
        assert_eq!(opened(resolve(&inv, &layout, workshop, false, "Tester", "..")), (archive, false));
        let raw = nav.address(&inv, &layout, workshop, true, "Tester");
        assert!(!raw.contains("Archive"));
        assert_eq!(opened(resolve(&inv, &layout, workshop, false, "Tester", &raw)), (workshop, true));
        assert_eq!(
            opened(resolve(&inv, &layout, workshop, true, "Tester", "..")),
            (Location::FullTree, true)
        );
        for mode in [false, true] {
            let libchild = Location::Folder(id(6));
            let address = nav.address(&inv, &layout, libchild, mode, "Tester");
            assert_eq!(
                opened(resolve(&inv, &layout, Location::Home, false, "Tester", &address)),
                (libchild, mode)
            );
        }
        inv.folders.get_mut(&id(4)).unwrap().state = FetchState::Unknown;
        assert!(matches!(resolve(&inv,&layout,Location::Home,false,"Tester",&display),Lookup::Load(id) if id==Uuid::from_u128(4)));
    }

    #[test]
    fn organized_scoped_search_collects_only_displayed_unfetched_children() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let archive = Location::Folder(id(4));
        assert!(layout.place(&inv, id(3), archive));
        inv.folders.get_mut(&id(3)).unwrap().state = FetchState::Unknown;
        inv.folders.get_mut(&id(8)).unwrap().state = FetchState::Unknown;
        let state = ExplorerState::at(archive);
        let (_, incomplete, requests) = search(&inv, &layout, &state, "Tester", "bench");
        assert!(incomplete);
        assert_eq!(requests, vec![id(3)]);
        let state = ExplorerState::at_mode(archive, true);
        let (_, incomplete, requests) = search(&inv, &layout, &state, "Tester", "bench");
        assert!(!incomplete);
        assert!(requests.is_empty());
    }
    #[test]
    fn organized_containing_folder_and_history_preserve_search_and_selection() {
        let inv = fixture();
        let mut layout = Preferences::default();
        let parent = Location::Folder(id(4));
        let item = Row::Item(id(20));
        assert!(layout.place(&inv, id(20), parent));
        let mut state = ExplorerState::default();
        state.here.query = "bench".into();
        state.here.selected.insert(item);
        assert_eq!(
            search_location(&inv, &layout, &mut state, item, "Tester").as_deref(),
            Some("Tester\\General Inventory\\Archive")
        );
        state.visit(parent, false);
        state.history(false);
        assert_eq!(state.here.query, "bench");
        assert!(state.here.selected.contains(&item));
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert!(state.here.selected.contains(&item));
        state.here.raw = true;
        assert!(
            search_location(&inv, &layout, &mut state, item, "Tester")
                .unwrap()
                .ends_with("\\Workshop")
        );
    }
}
