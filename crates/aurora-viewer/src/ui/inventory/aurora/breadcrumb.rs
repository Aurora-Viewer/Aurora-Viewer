//! Where the explorer is, as Windows Explorer's address bar and Dolphin's
//! location bar show it: the folders from the top down, each a link back
//! to it and a place to drop cargo into; the chevron after a folder lists
//! its subfolders to go into; a click in the empty part of the bar (or
//! Ctrl+L, Alt+D, F4) turns it into a path to type, completed from the
//! folders' names. Aurora difference: the Legacy Viewer's single folder
//! mode shows the folder's path as plain text (LLPanelMainInventory::
//! updateNavButtons, `path_text`).
//!
//! A path is the folders' names joined by `/`, the root written `~` as a
//! home folder (`~/Clothing/Shirts`; the root's translated name is read
//! too). A name may hold a `/` itself: the path is read from the top, each
//! level taking the longest name that fits (`resolve`).

use super::super::shortcuts::{self, Command};
use super::super::*;
use crate::ui::{menu, sheet, widgets};
use egui::{Sense, Vec2};

/// What separates two folders in a typed path.
pub(crate) const SEPARATOR: char = '/';
/// The inventory's root in a path, as a home folder on Linux and macOS:
/// `~/Clothing/Shirts`. It reads the same in every language, where the
/// root's name is translated ("My inventory", "Mon inventaire").
pub(crate) const HOME: &str = "~";
/// The most completions listed under a typed path.
const COMPLETIONS: usize = 10;
/// The last levels the bar keeps in sight; the others go under "…".
const LEVELS: usize = 4;

/// The bar while a path is typed in it.
#[derive(Debug, Default)]
pub(crate) struct PathEdit {
    pub text: String,
    /// The completion chosen with the arrows, if any.
    pick: Option<usize>,
    focus: bool,
    /// The typed path leads nowhere (shown until the text changes).
    missing: bool,
}

impl PathEdit {
    pub(crate) fn new(text: String) -> Self {
        Self {
            text,
            pick: None,
            focus: true,
            missing: false,
        }
    }
}

/// The top a path from `base` may start at: `base` itself, or the Library
/// beside the inventory's root.
fn tops(inv: &Inventory, base: Uuid) -> impl Iterator<Item = Uuid> {
    [Some(base), (base == inv.root && !inv.lib_root.is_nil()).then_some(inv.lib_root)]
        .into_iter()
        .flatten()
}

/// The folders from `base` (or the Library, from the root) down to
/// `folder`, the top first; empty when `folder` is under neither.
pub(crate) fn chain(inv: &Inventory, base: Uuid, folder: Uuid) -> Vec<Uuid> {
    let mut path = Vec::new();
    let mut id = folder;
    let mut seen = HashSet::new();
    while seen.insert(id) && inv.folders.contains_key(&id) {
        path.push(id);
        if id == base || id == inv.root || id == inv.lib_root {
            break;
        }
        match rules::parent(inv, id) {
            Some(parent) => id = parent,
            None => break,
        }
    }
    if !path.last().is_some_and(|top| tops(inv, base).any(|t| t == *top)) {
        return Vec::new();
    }
    path.reverse();
    path
}

/// The path of `folder` from `base`, `base`'s own name first:
/// "My Inventory/Clothing/Shirts".
pub(crate) fn path_text(inv: &Inventory, base: Uuid, folder: Uuid) -> String {
    let names: Vec<String> = chain(inv, base, folder)
        .into_iter()
        .map(|id| if id == inv.root { HOME.to_owned() } else { folder_name(inv, id) })
        .collect();
    names.join(&SEPARATOR.to_string())
}

/// The subfolders of `folder` in natural order of their names.
pub(crate) fn subfolders(inv: &Inventory, folder: Uuid) -> Vec<(Uuid, String)> {
    let mut list: Vec<(Uuid, String)> = inv
        .folders
        .get(&folder)
        .map(|f| f.children.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|id| inv.folders.contains_key(id))
        .map(|id| (*id, folder_name(inv, *id)))
        .collect();
    list.sort_by_cached_key(|(_, name)| crate::natural_order::key(name));
    list
}

/// `text` without its head `name`, ignoring case, if it starts with it.
fn strip_name<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let head = text.get(..name.len())?;
    (head.to_lowercase() == name.to_lowercase()).then(|| &text[name.len()..])
}

/// The folder a typed path names, read from `base` (its own name may lead
/// the path or not; a leading `/` is the top). Case does not count. None
/// when a level is not found.
pub(crate) fn resolve(inv: &Inventory, base: Uuid, text: &str) -> Option<Uuid> {
    let text = text.trim();
    let mut rest = text.trim_start_matches(SEPARATOR);
    let mut at = base;
    // `~` is the root, within a window whose top it is
    if base == inv.root
        && let Some(after) = rest
            .strip_prefix(HOME)
            .filter(|after| after.is_empty() || after.starts_with(SEPARATOR))
    {
        rest = after;
    }
    // a top's own name may lead the path: the window's top, or the Library
    for top in tops(inv, base) {
        if let Some(after) = strip_name(rest, &folder_name(inv, top)).filter(|after| after.is_empty() || after.starts_with(SEPARATOR)) {
            rest = after;
            at = top;
            break;
        }
    }
    loop {
        rest = rest.trim_start_matches(SEPARATOR);
        if rest.is_empty() {
            return Some(at);
        }
        // the longest name that ends where a level ends
        let next = subfolders(inv, at)
            .into_iter()
            .filter_map(|(id, name)| {
                strip_name(rest, &name)
                    .filter(|after| after.is_empty() || after.starts_with(SEPARATOR))
                    .map(|after| (id, name.len(), after))
            })
            .max_by_key(|(_, len, _)| *len)?;
        at = next.0;
        rest = next.2;
    }
}

/// The folders a typed path could go on with: the subfolders of its
/// complete part whose names start with its last part, as full paths.
pub(crate) fn completions(inv: &Inventory, base: Uuid, text: &str) -> Vec<(Uuid, String)> {
    let (head, tail) = match text.rfind(SEPARATOR) {
        Some(at) => (&text[..at], &text[at + 1..]),
        None => ("", text),
    };
    let Some(parent) = resolve(inv, base, head) else {
        return Vec::new();
    };
    // a bare name with no separator yet completes the top's subfolders
    let tail = tail.to_lowercase();
    let prefix = path_text(inv, base, parent);
    subfolders(inv, parent)
        .into_iter()
        .filter(|(_, name)| name.to_lowercase().starts_with(&tail))
        .take(COMPLETIONS)
        .map(|(id, name)| (id, format!("{prefix}{SEPARATOR}{name}")))
        .collect()
}

/// Opens the bar for typing, on the explorer's path.
pub(crate) fn edit(inv: &Inventory, st: &mut InventoryUi) {
    if st.tab == 0 {
        let base = super::base(inv, st);
        st.path_edit = Some(PathEdit::new(path_text(inv, base, st.browse)));
    }
}

/// Goes into `folder` from the bar.
fn enter(st: &mut InventoryUi, folder: Uuid) {
    st.search.clear();
    st.browse = folder;
    st.selection.clear();
    st.selected = None;
}

/// The bar, in the room the toolbar gives it.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    ui.spacing_mut().item_spacing.x = 4.0;
    if st.tab != 0 {
        ui.add(egui::Label::new(sheet::semibold(ui, super::toolbar::tab_name(st.tab), 13.0, p.ink)).truncate());
        return;
    }
    if st.path_edit.is_some() {
        typing(ui, p, inv, st);
        return;
    }
    // the empty part of the bar, under the folders: a click types a path
    let room = ui.available_rect_before_wrap();
    let blank = ui.interact(room, ui.scope_id().with("inventory_breadcrumb_blank"), Sense::click());
    let base = super::base(inv, st);
    let path = chain(inv, base, st.browse);
    let start = path.len().saturating_sub(LEVELS);
    if start > 0 {
        let more = ui
            .add(egui::Label::new(egui::RichText::new("…").size(13.0).color(p.muted)).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let mut chosen = None;
        menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            for folder in path[..start].iter().rev() {
                if menu::item(ui, p, "folder-filled", &folder_name(inv, *folder)) {
                    chosen = Some(*folder);
                }
            }
        });
        if let Some(folder) = chosen {
            enter(st, folder);
        }
        widgets::glyph(ui, "chevron-right", p.muted_dim, 12.0);
    }
    for (n, folder) in path[start..].iter().enumerate() {
        let index = start + n;
        let last = index + 1 == path.len();
        let name = folder_name(inv, *folder);
        let link = if last {
            ui.add(
                egui::Label::new(sheet::semibold(ui, &name, 13.0, p.ink))
                    .truncate()
                    .sense(Sense::click()),
            )
        } else {
            ui.add(egui::Label::new(egui::RichText::new(&name).size(13.0).color(p.muted)).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
        };
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_crumb", *folder)), link.rect));
        if link.clicked() && !last {
            enter(st, *folder);
        }
        // cargo dropped on a folder of the path goes into it
        drag::target(ui, p, &link, inv, *folder, st, prefs, facts, actions);
        segment_menu(&link, p, inv, st, *folder);
        let children = subfolders(inv, *folder);
        if children.is_empty() {
            continue;
        }
        let chevron = chevron(ui, p, *folder);
        let next = path.get(index + 1).copied();
        let mut chosen = None;
        menu::dropdown(&chevron, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (id, name) in &children {
                    if menu::check(ui, p, "folder-filled", name, Some(*id) == next, true) {
                        chosen = Some(*id);
                    }
                }
            });
        });
        if let Some(folder) = chosen {
            enter(st, folder);
        }
    }
    if blank.clicked() {
        edit(inv, st);
    }
    blank.on_hover_text(shortcuts::tip(ui.ctx(), aurora_i18n::tr!("inventory-path-edit"), Command::EditPath));
}

/// The small button after a folder that lists its subfolders.
fn chevron(ui: &mut egui::Ui, p: &Palette, folder: Uuid) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 20.0), Sense::hover());
    let response = ui.interact(rect, ui.scope_id().with(("inventory_crumb_chevron", folder)), Sense::click());
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_crumb_chevron", folder)), rect));
    let hot = response.hovered() || response.has_focus();
    if hot {
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            p.raised.lerp_to_gamma(p.ink, 0.16),
        );
    }
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    widgets::paint_glyph(
        ui.painter(),
        if open { "chevron-down" } else { "chevron-right" },
        rect.center(),
        12.0,
        if hot { p.ink } else { p.muted_dim },
    );
    response.on_hover_text(aurora_i18n::tr!("inventory-path-subfolders"))
}

/// A folder of the path: copy its path, type a path, open it in a window.
fn segment_menu(link: &egui::Response, p: &Palette, inv: &Inventory, st: &mut InventoryUi, folder: Uuid) {
    let mut chosen = 0;
    menu::context_menu(link, p, |ui| {
        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("inventory-copy-path")) {
            chosen = 1;
        }
        let keys = shortcuts::label(ui.ctx(), Command::EditPath).unwrap_or_default();
        if menu::item_keys(ui, p, "pencil-filled", aurora_i18n::tr!("inventory-path-edit"), &keys) {
            chosen = 2;
        }
        if menu::item(
            ui,
            p,
            "external-link-filled",
            aurora_i18n::tr!("ui-inventory-context-show-in-a-new-window"),
        ) {
            chosen = 3;
        }
    });
    match chosen {
        1 => link.ctx.copy_text(path_text(inv, super::base(inv, st), folder)),
        2 => edit(inv, st),
        3 => st.open_folder_window(folder),
        _ => {}
    }
}

/// The bar as a text field, with its completions under it.
fn typing(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi) {
    let base = super::base(inv, st);
    let Some(edit) = st.path_edit.as_mut() else { return };
    let options = completions(inv, base, &edit.text);
    // the arrows and Tab work the list before the field sees them
    let (down, up, tab, enter_key, escape) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if !options.is_empty() {
        let last = options.len() - 1;
        if down {
            edit.pick = Some(edit.pick.map_or(0, |n| (n + 1).min(last)));
        }
        if up {
            edit.pick = edit.pick.and_then(|n| n.checked_sub(1));
        }
    }
    if tab && let Some((_, path)) = options.get(edit.pick.unwrap_or(0)) {
        edit.text = format!("{path}{SEPARATOR}");
        edit.pick = None;
        edit.focus = true;
    }
    let field = egui::TextEdit::singleline(&mut edit.text)
        .id(ui.scope_id().with("inventory_path_field"))
        .desired_width(ui.available_width())
        .text_color(if edit.missing { p.danger } else { p.ink })
        .font(egui::FontId::proportional(13.0));
    let r = ui.add(field);
    if std::mem::take(&mut edit.focus) {
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let end = egui::text::CCursor::new(edit.text.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.changed() {
        edit.missing = false;
        edit.pick = None;
    }
    let mut chosen = None;
    let mut popup = None;
    if !options.is_empty() {
        let below = egui::pos2(r.rect.left(), r.rect.bottom() + 2.0);
        let shown = egui::Area::new(ui.scope_id().with("inventory_path_completions"))
            .order(egui::Order::Foreground)
            .fixed_pos(below)
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(r.rect.width().min(420.0));
                    for (n, (id, path)) in options.iter().enumerate() {
                        let name = path.rsplit(SEPARATOR).next().unwrap_or(path);
                        if menu::check(ui, p, "folder-filled", name, edit.pick == Some(n), true) {
                            chosen = Some(*id);
                        }
                    }
                });
            });
        popup = Some(shown.response.rect);
    }
    if escape {
        st.path_edit = None;
        clipboard::request_focus(ui, st);
        return;
    }
    if enter_key {
        let target = match edit.pick.and_then(|n| options.get(n)) {
            Some((id, _)) => Some(*id),
            None => resolve(inv, base, &edit.text),
        };
        match target {
            Some(folder) => chosen = Some(folder),
            None => {
                edit.missing = true;
                edit.focus = true;
            }
        }
    }
    if let Some(folder) = chosen {
        st.path_edit = None;
        enter(st, folder);
        clipboard::request_focus(ui, st);
    } else if ui.input(|i| i.pointer.any_pressed())
        && !r.contains_pointer()
        && !popup.is_some_and(|rect| ui.rect_contains_pointer(rect) || ui.ctx().pointer_hover_pos().is_some_and(|pos| rect.contains(pos)))
    {
        // a click elsewhere leaves the path as it was (the completions'
        // clicks are read above, before this)
        st.path_edit = None;
    } else if !r.has_focus() && !enter_key && ui.memory(|m| m.focused().is_none_or(|f| f == clipboard::focus_id(st))) {
        // the field keeps the keyboard while it is open (the list asks for
        // it back when it is revealed)
        r.request_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};

    fn folder(inv: &mut Inventory, id: u128, parent: u128, name: &str) -> Uuid {
        let id = Uuid::from_u128(id);
        let parent = Uuid::from_u128(parent);
        inv.folders.insert(
            id,
            Folder {
                info: aurora_net::inventory::InvFolder {
                    id,
                    parent,
                    name: name.into(),
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
        id
    }

    fn tree() -> (Inventory, Uuid) {
        let mut inv = Inventory::default();
        let root = folder(&mut inv, 1, 0, "My Inventory");
        inv.root = root;
        folder(&mut inv, 2, 1, "Clothing");
        folder(&mut inv, 3, 2, "Shirts");
        folder(&mut inv, 4, 2, "Shoes");
        folder(&mut inv, 5, 1, "AC/DC");
        folder(&mut inv, 6, 5, "Tour");
        folder(&mut inv, 7, 1, "AC");
        folder(&mut inv, 8, 1, "Item 10");
        folder(&mut inv, 9, 1, "Item 9");
        (inv, root)
    }

    #[test]
    fn a_path_names_the_folders_from_the_top() {
        let (inv, root) = tree();
        // the root is ~, in every language
        assert_eq!(path_text(&inv, root, Uuid::from_u128(3)), "~/Clothing/Shirts");
        assert_eq!(path_text(&inv, root, root), "~");
        // a folder window's path starts at its folder
        assert_eq!(path_text(&inv, Uuid::from_u128(2), Uuid::from_u128(3)), "Clothing/Shirts");
        assert!(chain(&inv, Uuid::from_u128(5), Uuid::from_u128(3)).is_empty());
    }

    #[test]
    fn a_typed_path_is_found_whatever_its_case_and_its_top() {
        let (inv, root) = tree();
        let shirts = Some(Uuid::from_u128(3));
        assert_eq!(resolve(&inv, root, "My Inventory/Clothing/Shirts"), shirts);
        assert_eq!(resolve(&inv, root, "clothing/SHIRTS/"), shirts);
        assert_eq!(resolve(&inv, root, "/Clothing/Shirts"), shirts);
        assert_eq!(resolve(&inv, root, "  "), Some(root));
        assert_eq!(resolve(&inv, root, "My Inventory"), Some(root));
        // ~ is the root, as a home folder
        assert_eq!(resolve(&inv, root, "~/Clothing/Shirts"), shirts);
        assert_eq!(resolve(&inv, root, "~"), Some(root));
        assert_eq!(resolve(&inv, root, "~/"), Some(root));
        // a folder window has no ~ above its top
        assert_eq!(resolve(&inv, Uuid::from_u128(2), "~/Shirts"), None);
        assert_eq!(resolve(&inv, root, "Clothing/Hats"), None);
    }

    #[test]
    fn the_library_has_a_path_of_its_own() {
        let (mut inv, root) = tree();
        let lib = folder(&mut inv, 50, 0, "Library");
        inv.lib_root = lib;
        let clothing = folder(&mut inv, 51, 50, "Clothing");
        let top = folder_name(&inv, lib);
        assert_eq!(path_text(&inv, root, clothing), format!("{top}/Clothing"));
        assert_eq!(resolve(&inv, root, &format!("{top}/clothing")), Some(clothing));
        // the inventory's own Clothing is still the one without the top
        assert_eq!(resolve(&inv, root, "Clothing"), Some(Uuid::from_u128(2)));
    }

    #[test]
    fn a_name_holding_the_separator_is_read_whole() {
        let (inv, root) = tree();
        assert_eq!(resolve(&inv, root, "AC/DC/Tour"), Some(Uuid::from_u128(6)));
        assert_eq!(resolve(&inv, root, "AC/DC"), Some(Uuid::from_u128(5)));
        assert_eq!(resolve(&inv, root, "AC"), Some(Uuid::from_u128(7)));
    }

    #[test]
    fn completions_list_the_matching_subfolders_in_natural_order() {
        let (inv, root) = tree();
        let names = |text: &str| completions(&inv, root, text).into_iter().map(|(_, p)| p).collect::<Vec<_>>();
        assert_eq!(names("Clothing/sh"), vec!["~/Clothing/Shirts", "~/Clothing/Shoes"]);
        assert_eq!(names("~/Item"), vec!["~/Item 9", "~/Item 10"]);
        // the root's translated name still leads to the same folders
        let top = folder_name(&inv, root);
        assert_eq!(names(&format!("{top}/Item")), vec!["~/Item 9", "~/Item 10"]);
        assert!(names("Nowhere/x").is_empty());
    }
}
