//! Real inventory creation initiated by an Explorer tab. Only a confirmed folder
//! can acquire a local placement in a legacy virtual folder.
use super::*;
use aurora_i18n::tr;
use aurora_net::inventory::operations::NewItem;

#[derive(Debug, Clone)]
pub struct ExplorerFolderRequest {
    pub(crate) token: Uuid,
    pub(crate) root: Uuid,
    pub(crate) parent: Uuid,
    pub(crate) name: String,
    pub(crate) legacy: Option<Uuid>,
    raw: bool,
    pub(crate) item: Option<NewItem>,
    pub(crate) create_backing: bool,
    pub(crate) home_child: bool,
}

fn writable(inv: &Inventory, id: Uuid) -> bool {
    !id.is_nil()
        && model::personal_contents(inv, id)
        && !model::client_managed(inv, id)
        && !rules::in_type(inv, id, 16)
        && rules::destination(inv, id)
}

impl ExplorerFolderRequest {
    pub(crate) fn matches_item(&self, it: &aurora_net::inventory::InvItem, agent: Uuid) -> bool {
        self.item.is_some_and(|kind| {
            let (asset, inv, subtype) = kind.types();
            !it.id.is_nil()
                && !agent.is_nil()
                && it.owner == agent
                && it.parent == self.parent
                && it.asset_type == i32::from(asset)
                && it.inv_type == i32::from(inv)
                && (!matches!(kind, NewItem::Wearable(_) | NewItem::Settings(_)) || it.flags as u8 == subtype)
        })
    }

    pub(crate) fn change(&self, inv: &Inventory) -> Result<aurora_net::inventory::operations::Mutation, String> {
        self.validate(inv)?;
        if self.create_backing {
            if !inv.folders.get(&inv.root).is_some_and(|f| f.state == FetchState::Fetched) {
                return Err(tr!("explorer-new-home-loading").into());
            }
            let base = crate::text_validation::default_name("explorer-new-home-container");
            let existing: HashSet<_> = inv
                .folders
                .values()
                .filter(|f| !f.library && f.info.parent == inv.root)
                .map(|f| f.info.name.to_lowercase())
                .collect();
            let name = if existing.contains(&base.to_lowercase()) {
                (2..)
                    .map(|n| format!("{base} ({n})"))
                    .find(|n| !existing.contains(&n.to_lowercase()))
                    .unwrap()
            } else {
                base.into()
            };
            rules::create_folder(self.parent, &name)
        } else {
            rules::create_folder(self.parent, &self.name)
        }
    }

    pub(crate) fn validate(&self, inv: &Inventory) -> Result<(), String> {
        if self.root != inv.root || !writable(inv, self.parent) {
            return Err(tr!("explorer-create-unavailable").into());
        }
        checked_name(&self.name)
    }
}

fn checked_name(name: &str) -> Result<(), String> {
    crate::text_validation::require(crate::text_validation::Policy::PrintableNoPipe, name)?;
    if name.trim().is_empty() {
        return Err(tr!("world-inventory-actions-enter-a-valid-name").into());
    }
    if name.trim().len() > 63 {
        return Err(tr!("explorer-create-length").into());
    }
    Ok(())
}

pub(super) struct Draft {
    row_id: Uuid,
    kind: Option<NewItem>,
    default_name: String,
    create_backing: bool,
    origin: Location,
    raw: bool,
    parent: Option<Uuid>,
    name: String,
    focus: bool,
    error: String,
    pub(super) pending: Option<Uuid>,
}

pub(super) fn fixed_parent(place: Location, inv: &Inventory) -> Option<Uuid> {
    match place {
        Location::Home | Location::MyFolders | Location::FullTree => Some(inv.root),
        Location::Folder(id) => Some(id),
        _ => None,
    }
}

pub(super) fn available(inv: &Inventory, place: Location) -> bool {
    match place {
        Location::Library | Location::Group(Group::System) => false,
        Location::Group(_) | Location::Custom(_) => writable(inv, inv.root),
        _ => fixed_parent(place, inv).is_some_and(|id| writable(inv, id)),
    }
}

// Always require an explicit selection for a place with no physical parent UUID.
// A real folder represented by an alias keeps its canonical path here.
pub(super) fn choices(inv: &Inventory, layout: &Preferences, place: Location) -> Vec<Uuid> {
    let mut targets = layout.children(inv, place);
    targets.retain(|id| *id != inv.root && inv.folders.contains_key(id) && writable(inv, *id));
    targets.sort_by_cached_key(|id| super::super::inventory_path(inv, *id));
    targets.dedup();
    if writable(inv, inv.root) {
        targets.push(inv.root);
    }
    targets
}

pub(in super::super) fn begin(inv: &Inventory, layout: &Preferences, state: &mut ExplorerState) {
    let place = state.here.location;
    if !available(inv, place) || !layout.available(inv, place) || state.folder_draft.as_ref().is_some_and(|d| d.pending.is_some()) {
        return;
    }
    if let Some(draft) = state.folder_draft.as_mut() {
        draft.focus = true;
        return;
    }
    let home = place == Location::Home && !state.here.raw;
    let backing = home.then(|| model::home_folder(inv, layout)).flatten();
    start(
        state,
        backing.or_else(|| fixed_parent(place, inv)),
        None,
        crate::text_validation::default_name("common-new-folder"),
        home && backing.is_none(),
    );
}

pub(super) fn start(state: &mut ExplorerState, parent: Option<Uuid>, kind: Option<NewItem>, default: &str, create_backing: bool) {
    if state.folder_draft.is_some() {
        return;
    }
    let location = state.here.location;
    state.visit(location, state.here.raw);
    state.clear_filter();
    let default = crate::text_validation::generated_name(default, 63);
    let row_id = Uuid::new_v4();
    state.folder_draft = Some(Draft {
        row_id,
        kind,
        default_name: default.clone(),
        create_backing,
        origin: location,
        raw: state.here.raw,
        parent,
        name: default,
        focus: true,
        error: String::new(),
        pending: None,
    });
    state.rows_key = None;
    state.here.clear_selection();
    state.reveal_row = state.folder_draft.as_ref().and_then(|d| d.row(state));
    if let Some(row) = state.reveal_row {
        state.here.selected.insert(row);
    }
}

impl Draft {
    pub(super) fn left_origin(&self, state: &ExplorerState) -> bool {
        self.origin != state.here.location || self.raw != state.here.raw
    }

    pub(super) fn discard_on_navigation(&self) -> bool {
        self.pending.is_none() && (self.parent.is_none() || !self.error.is_empty())
    }

    pub(super) fn row(&self, state: &ExplorerState) -> Option<Row> {
        (self.parent.is_some() && self.origin == state.here.location && self.raw == state.here.raw).then_some(if self.kind.is_some() {
            Row::Item(self.row_id)
        } else {
            Row::Place(Location::Folder(self.row_id))
        })
    }

    pub(super) fn initial_name(&self) -> &str {
        &self.default_name
    }

    pub(super) fn item_type(&self) -> Option<aurora::rows::ItemType> {
        self.kind.map(|kind| {
            let (asset, ty, subtype) = kind.types();
            aurora::rows::ItemType::Asset(aurora::rows::AssetClass::new(i32::from(asset), i32::from(ty), subtype))
        })
    }

    pub(super) fn display_name(&self) -> &str {
        &self.name
    }

    pub(super) fn glyph(&self) -> &'static str {
        match self.kind {
            None => "Inv_FolderClosed",
            Some(NewItem::Script) => "Inv_Script",
            Some(NewItem::Note) => "Inv_Notecard",
            Some(NewItem::Gesture) => "Inv_Gesture",
            Some(NewItem::Landmark) => "Inv_Landmark",
            Some(NewItem::Material) => "Inv_Material",
            Some(NewItem::Settings(_)) => "Inv_Settings",
            Some(NewItem::Wearable(n)) => crate::ui::inventory::wearable_icon(if n < 4 { 13 } else { 5 }, n as u32),
        }
    }
}

fn submit(draft: &mut Draft, inv: &Inventory, busy: bool) -> Result<ExplorerFolderRequest, String> {
    if draft.pending.is_some() || (busy && draft.origin != Location::Home) {
        return Err(tr!("ui-inventory-wait-for-the-current-change-to-finish").into());
    }
    let parent = draft.parent.ok_or_else(|| tr!("explorer-create-choose-parent").to_owned())?;
    checked_name(&draft.name)?;
    let request = ExplorerFolderRequest {
        token: Uuid::new_v4(),
        home_child: draft.origin == Location::Home && !draft.raw,
        create_backing: draft.create_backing,
        item: draft.kind,
        raw: draft.raw,
        root: inv.root,
        parent,
        name: draft.name.trim().to_owned(),
        legacy: match draft.origin {
            Location::Custom(id) => Some(id),
            _ => None,
        },
    };
    request.validate(inv)?;
    draft.pending = Some(request.token);
    Ok(request)
}

/// Only multi-destination views need a parent picker. Naming always happens in the actual cell.
pub(super) fn tools(ui: &mut Ui, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState) {
    let Some(draft) = state.folder_draft.as_mut() else { return };
    if draft.origin != state.here.location || draft.parent.is_some() {
        return;
    }
    let targets = choices(inv, layout, draft.origin);
    egui::ComboBox::from_id_salt("explorer-real-parent")
        .selected_text(tr!("explorer-create-choose-parent"))
        .show_ui(ui, |ui| {
            for id in targets {
                if ui.selectable_label(false, super::super::inventory_path(inv, id)).clicked() {
                    draft.parent = Some(id);
                    state.rows_key = None;
                }
            }
        });
    if draft.parent.is_some() {
        state.reveal_row = state.folder_draft.as_ref().and_then(|d| d.row(state));
    }
    if ui.small_button(tr!("common-cancel")).clicked() {
        state.folder_draft = None;
        state.rows_key = None;
    }
}

/// Clicking another folder, tab or window completes a valid inline creation, including one
/// whose editor has just scrolled out of the visible cell range. Failed drafts require an explicit retry.
pub(in super::super) fn leave(state: &mut ExplorerState, inv: &Inventory, busy: bool, actions: &mut Vec<InvAction>) {
    let Some(draft) = state.folder_draft.as_mut() else { return };
    if draft.pending.is_some() || draft.parent.is_none() || !draft.error.is_empty() {
        return;
    }
    if checked_name(&draft.name).is_err() {
        draft.name.clone_from(&draft.default_name);
    }
    match submit(draft, inv, busy) {
        Ok(origin) => actions.push(InvAction::CreateExplorerFolder(origin)),
        Err(reason) => {
            draft.error = reason.clone();
            state.status = reason;
            draft.focus = true;
        }
    }
}

pub(super) fn inline_create(ui: &mut Ui, inv: &Inventory, state: &mut ExplorerState, native: &InventoryUi, actions: &mut Vec<InvAction>) {
    let Some(draft) = state.folder_draft.as_mut() else { return };
    if draft.pending.is_some() || draft.parent.is_none() {
        return;
    }
    let first = std::mem::take(&mut draft.focus);
    let response = super::super::super::widgets::restricted_text_field(
        ui,
        true,
        &mut draft.name,
        super::super::super::widgets::TextPolicy::PrintableNoPipe,
        |buf, ui| {
            egui::TextEdit::singleline(buf)
                .id(egui::Id::unique(("explorer-new-name", draft.row_id)))
                .char_limit(63)
                .desired_width(ui.available_width())
                .min_size(vec2(1.0, 18.0))
        },
    );
    if first {
        response.request_focus();
        if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            edit.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(draft.name.chars().count()),
            )));
            edit.store(ui.ctx(), response.id);
        }
    } else {
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        if escape && !draft.error.is_empty() {
            state.folder_draft = None;
            state.rows_key = None;
            state.here.clear_selection();
            state.status.clear();
            return;
        }
        if response.changed() {
            draft.error.clear();
        }
        if escape {
            draft.name.clone_from(&draft.default_name);
        }
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        if escape || enter || (!response.has_focus() && draft.error.is_empty()) {
            match submit(draft, inv, native.pending.is_some()) {
                Ok(request) => {
                    state.status = tr!("explorer-create-in", path = super::super::inventory_path(inv, request.parent));
                    actions.push(InvAction::CreateExplorerFolder(request));
                }
                Err(reason) => {
                    draft.error = reason.clone();
                    state.status = reason;
                    draft.focus = true;
                }
            }
        }
    }
}

pub(in super::super) fn confirmed_destination(
    inv: &Inventory,
    layout: &mut Preferences,
    request: &ExplorerFolderRequest,
    result: Result<Uuid, &str>,
) -> Location {
    if let (Some(legacy), Ok(id)) = (request.legacy, result) {
        let place = Location::Custom(legacy);
        if layout.move_entries(inv, &[id], &[], place) {
            return place;
        }
    }
    if let (Some(_), Ok(id)) = (request.item, result) {
        return if request.raw {
            Location::Folder(request.parent)
        } else {
            layout.parent(inv, id)
        };
    }
    let parent = match result.ok().and_then(|id| inv.folders.get(&id)) {
        Some(folder) if !request.raw => return layout.parent(inv, folder.info.id),
        Some(folder) => folder.info.parent,
        None => request.parent,
    };
    if parent == inv.root {
        Location::Home
    } else {
        Location::Folder(parent)
    }
}

pub(in super::super) fn finish(
    state: &mut ExplorerState,
    request: &ExplorerFolderRequest,
    destination: Location,
    result: &Result<Uuid, &str>,
    demo: bool,
) -> bool {
    let Some(draft) = state.folder_draft.as_mut().filter(|d| d.pending == Some(request.token)) else {
        return false;
    };
    let still_here = (draft.origin, draft.raw) == (state.here.location, state.here.raw);
    let raw = draft.raw;
    match result {
        Ok(id) => {
            state.folder_draft = None;
            if still_here {
                let destination = if raw && destination == Location::Home {
                    Location::FullTree
                } else {
                    destination
                };
                state.visit(destination, raw);
                let row = if request.item.is_some() {
                    Row::Item(*id)
                } else {
                    Row::Place(Location::Folder(*id))
                };
                state.select_and_reveal(row);
            }
            state.status = if demo {
                tr!("explorer-create-demo-done", name = request.name.as_str())
            } else {
                tr!("explorer-create-done", name = request.name.as_str())
            };
        }
        Err(reason) if still_here => {
            draft.pending = None;
            state.status = (*reason).to_owned();
            draft.error = (*reason).to_owned();
            draft.focus = true;
        }
        Err(reason) => {
            state.folder_draft = None;
            state.status = (*reason).to_owned();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn add(inv: &mut Inventory, n: u128, parent: u128, name: &str, kind: i32) {
        let id = Uuid::from_u128(n);
        let parent = Uuid::from_u128(parent);
        inv.folders.insert(
            id,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(p) = inv.folders.get_mut(&parent) {
            p.children.push(id);
        }
        inv.generation += 1;
    }

    #[test]
    fn organized_create_validates_the_live_real_parent_and_stock_name_policy() {
        let mut inv = super::super::tests::fixture();
        let layout = Preferences::default();
        for (n, p, name, kind) in [
            (11, 1, "Trash", 14),
            (12, 11, "Trashed", -1),
            (13, 1, "#Aurora", -1),
            (14, 13, "Client state", -1),
            (15, 1, "Listings", 53),
            (16, 1, "Clothing", 5),
            (17, 1, "Current outfit", 46),
            (18, 99, "Missing ancestor", -1),
        ] {
            add(&mut inv, n, p, name, kind);
        }
        for number in [1, 3, 5, 7, 16] {
            assert!(writable(&inv, id(number)), "{number}");
        }
        for number in [2, 6, 11, 12, 13, 14, 15, 17, 18, 99] {
            assert!(!writable(&inv, id(number)), "{number}");
        }
        let group = Location::Group(Group::Appearance);
        let mut tab = ExplorerState::at(group);
        begin(&inv, &layout, &mut tab);
        let draft = tab.folder_draft.as_mut().unwrap();
        draft.name = "Boots".into();
        assert!(draft.parent.is_none());
        assert!(submit(draft, &inv, false).is_err());
        let parents = choices(&inv, &layout, group);
        assert!(parents.contains(&inv.root) && parents.contains(&id(16)) && !parents.contains(&id(17)));
        draft.parent = Some(id(16));
        for invalid in ["   ", "a|b", "bad\nname", "東京", &"x".repeat(64)] {
            draft.name = invalid.into();
            assert!(submit(draft, &inv, false).is_err(), "{invalid}");
        }
        draft.name = " Boots ".into();
        let origin = submit(draft, &inv, false).unwrap();
        assert_eq!((origin.parent, origin.name.as_str()), (id(16), "Boots"));
        assert!(submit(draft, &inv, false).is_err());
        inv.folders.get_mut(&id(16)).unwrap().info.parent = id(11);
        assert!(
            origin.change(&inv).is_err(),
            "the parent moved into Trash before the app handled the request"
        );
    }

    #[test]
    fn organized_create_selects_a_new_client_root_in_system_and_in_the_raw_tree() {
        for raw in [false, true] {
            let mut inv = super::super::tests::fixture();
            let mut layout = Preferences::default();
            let start = if raw { Location::FullTree } else { Location::MyFolders };
            let mut tab = ExplorerState::at_mode(start, raw);
            begin(&inv, &layout, &mut tab);
            let draft = tab.folder_draft.as_mut().unwrap();
            draft.name = "#RLV".into();
            let request = submit(draft, &inv, false).unwrap();
            let actual = id(901);
            add(&mut inv, 901, 1, "#RLV", -1);
            let destination = confirmed_destination(&inv, &mut layout, &request, Ok(actual));
            assert!(finish(&mut tab, &request, destination, &Ok(actual), false));
            let expected = if raw { Location::FullTree } else { Location::Group(Group::System) };
            assert_eq!(tab.here.location, expected);
            refresh_rows(&inv, &mut layout, &mut tab, "Tester");
            let row = Row::Place(Location::Folder(actual));
            assert!(tab.here.selected.contains(&row) && tab.rows.contains(&row));
            assert_eq!(inv.folders[&actual].info.parent, inv.root);
        }
    }

    #[test]
    fn explorer_creation_only_confirms_the_expected_owner_type_subtype_and_destination() {
        let agent = id(13);
        let mut tab = ExplorerState::at(Location::Folder(id(3)));
        let inv = super::super::tests::fixture();
        start(&mut tab, Some(id(3)), Some(NewItem::Wearable(4)), "New Shirt", false);
        let req = submit(tab.folder_draft.as_mut().unwrap(), &inv, false).unwrap();
        let good = aurora_net::inventory::InvItem {
            id: id(600),
            parent: id(3),
            owner: agent,
            asset_type: 5,
            inv_type: 18,
            flags: 4,
            ..Default::default()
        };
        assert!(req.matches_item(&good, agent));
        for bad in [
            aurora_net::inventory::InvItem {
                id: Uuid::nil(),
                ..good.clone()
            },
            aurora_net::inventory::InvItem {
                parent: id(4),
                ..good.clone()
            },
            aurora_net::inventory::InvItem {
                owner: id(15),
                ..good.clone()
            },
            aurora_net::inventory::InvItem {
                asset_type: 13,
                ..good.clone()
            },
            aurora_net::inventory::InvItem {
                inv_type: 10,
                ..good.clone()
            },
            aurora_net::inventory::InvItem { flags: 6, ..good },
        ] {
            assert!(!req.matches_item(&bad, agent));
        }
    }

    #[test]
    fn home_creates_one_independent_backing_and_then_a_real_nested_folder() {
        let mut inv = super::super::tests::fixture();
        let mut layout = Preferences::default();
        add(&mut inv, 800, 1, tr!("explorer-new-home-container"), -1);
        add(&mut inv, 801, 1, &format!("{} (2)", tr!("explorer-new-home-container")), -1);
        let mut state = ExplorerState::at(Location::Home);
        begin(&inv, &layout, &mut state);
        let draft = state.folder_draft.as_mut().unwrap();
        draft.name = "My project".into();
        let req = submit(draft, &inv, false).unwrap();
        assert!(req.home_child && req.create_backing);
        let change = req.change(&inv).unwrap();
        assert!(
            matches!(&change.operations[..], [aurora_net::inventory::operations::Operation::Folder{parent,name,..}] if *parent==inv.root && name==&format!("{} (3)",tr!("explorer-new-home-container")))
        );
        add(&mut inv, 802, 1, "Home Folders (3)", -1);
        layout.set_home_folder(id(802));
        let mut other = ExplorerState::at(Location::Home);
        begin(&inv, &layout, &mut other);
        let child = submit(other.folder_draft.as_mut().unwrap(), &inv, false).unwrap();
        assert_eq!(child.parent, id(802));
        assert!(child.home_child && !child.create_backing);
        add(&mut inv, 803, 802, &child.name, -1);
        let dest = confirmed_destination(&inv, &mut layout, &child, Ok(id(803)));
        assert_eq!(dest, Location::Home);
        assert!(finish(&mut other, &child, dest, &Ok(id(803)), false));
        assert_eq!(
            other.here.selected.iter().copied().collect::<Vec<_>>(),
            [Row::Place(Location::Folder(id(803)))]
        );
    }

    #[test]
    fn organized_create_in_legacy_folder_waits_for_actual_uuid_and_keeps_its_physical_parent() {
        let mut inv = super::super::tests::fixture();
        let mut layout = Preferences::default();
        let legacy = layout.create_custom(&inv, Location::MyFolders, "Legacy shortcuts").unwrap();
        let mut tab = ExplorerState::at(legacy);
        begin(&inv, &layout, &mut tab);
        let draft = tab.folder_draft.as_mut().unwrap();
        assert!(draft.parent.is_none());
        draft.parent = Some(inv.root);
        draft.name = "Textures to sort".into();
        let request = submit(draft, &inv, false).unwrap();
        let plan = request.change(&inv).unwrap();
        let [aurora_net::inventory::operations::Operation::Folder { id: asked, .. }] = &plan.operations[..] else {
            panic!()
        };
        let actual = id(900);
        assert_ne!(actual, *asked);
        assert!(!layout.children(&inv, legacy).contains(&actual));
        assert_eq!(
            confirmed_destination(&inv, &mut layout, &request, Err("server failed")),
            Location::Home
        );
        add(&mut inv, 900, 1, "Textures to sort", -1);
        let destination = confirmed_destination(&inv, &mut layout, &request, Ok(actual));
        assert_eq!(destination, legacy);
        assert!(finish(&mut tab, &request, destination, &Ok(actual), false));
        assert!(tab.here.selected.contains(&Row::Place(Location::Folder(actual))));
        assert!(layout.children(&inv, legacy).contains(&actual));
        assert_eq!(inv.folders[&actual].info.parent, inv.root);
        assert!(!inv.folders.contains_key(asked));
        assert!(!finish(&mut tab, &request, destination, &Ok(actual), false));
    }
}
