//! Capture the main menu's explicitly named commands and dispatch through its
//! original handlers. Moving a command never copies its permission checks or
//! action logic. Collection has no input and execution activates one enabled row.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use egui::{Id, Response, Ui};

use super::{Drag, Edit, ToolbarSettings, ToolbarUi};
use crate::theme::Palette;
use crate::ui::{menu, skin::ToolButton};

pub const PREFIX: &str = "menu:";

#[derive(Clone, Debug)]
pub struct Entry {
    pub button: ToolButton,
    pub parent: String,
    pub group: bool,
    pub info: bool,
    pub separator: bool,
    pub enabled: bool,
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    pub entries: Vec<Entry>,
}

impl Catalogue {
    pub fn find(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.button.command == id)
    }

    pub fn children<'a>(&'a self, parent: &str, layout: &BTreeMap<String, Vec<String>>) -> Vec<&'a Entry> {
        let moved: HashSet<&str> = layout.values().flatten().map(String::as_str).collect();
        let mut children: Vec<_> = layout.get(parent).into_iter().flatten().filter_map(|id| self.find(id)).collect();
        children.extend(
            self.entries
                .iter()
                .filter(|entry| entry.parent == parent && !moved.contains(entry.button.command.as_str())),
        );
        children
    }

    pub fn can_place(&self, command: &str, parent: &str, layout: &BTreeMap<String, Vec<String>>) -> bool {
        // These are interactive surfaces, not menu actions. Their toolbar
        // versions continue to work on all four edges.
        if matches!(command, "aurora_menu" | "speak" | "favorites")
            || super::structure::is_separator(command)
            || super::structure::is_block(command)
        {
            return false;
        }
        if self.find(command).is_none() || (!parent.is_empty() && !self.find(parent).is_some_and(|entry| entry.group)) {
            return false;
        }
        let mut ancestor = parent;
        let mut seen = HashSet::new();
        while !ancestor.is_empty() {
            if ancestor == command || !seen.insert(ancestor) {
                return false;
            }
            ancestor = layout
                .iter()
                .find(|(_, items)| items.iter().any(|item| item == ancestor))
                .map(|(parent, _)| parent.as_str())
                .or_else(|| self.find(ancestor).map(|entry| entry.parent.as_str()))
                .unwrap_or("");
        }
        true
    }
}

#[derive(Default)]
struct Session {
    path: Vec<String>,
    catalogue: Catalogue,
    target: Option<String>,
    fired: bool,
    disabled: usize,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static CURRENT: RefCell<Arc<Catalogue>> = RefCell::new(Arc::default());
}

pub fn scanning() -> bool {
    SESSION.with_borrow(Option::is_some)
}

/// Explicit keys are independent of translated labels and source line numbers.
pub fn identified<R>(ui: &mut Ui, key: impl AsRef<str>, body: impl FnOnce(&mut Ui) -> R) -> R {
    SESSION.with_borrow_mut(|session| {
        if let Some(session) = session {
            session.path.push(key.as_ref().to_owned());
        }
    });
    let result = body(ui);
    SESSION.with_borrow_mut(|session| {
        if let Some(session) = session {
            session.path.pop();
        }
    });
    result
}

pub fn scope(ui: &mut Ui, target: Option<String>, body: impl FnOnce(&mut Ui)) -> Catalogue {
    SESSION.with_borrow_mut(|session| {
        *session = Some(Session {
            target,
            ..Default::default()
        })
    });
    body(ui);
    SESSION.with_borrow_mut(|session| session.take().map(|session| session.catalogue).unwrap_or_default())
}

/// Synthetic responses never receive pointer/keyboard input from the hidden UI.
pub fn row(ui: &Ui, icon: &str, label: &str, enabled: bool, group: bool, checked: Option<bool>) -> Response {
    let (id, clicked) = SESSION.with_borrow_mut(|session| {
        let Some(session) = session else { return (String::new(), false) };
        let id = format!("{PREFIX}{}", session.path.join("/"));
        let parent = session.path[..session.path.len().saturating_sub(1)].join("/");
        let parent = if parent.is_empty() { parent } else { format!("{PREFIX}{parent}") };
        let enabled = enabled && session.disabled == 0;
        if !session.path.is_empty() && !session.catalogue.entries.iter().any(|entry| entry.button.command == id) {
            session.catalogue.entries.push(Entry {
                button: ToolButton {
                    command: id.clone(),
                    icon: icon.to_owned(),
                    tooltip: aurora_i18n::key_of(label).map_or_else(|| label.to_owned(), |key| format!("@{key}")),
                },
                parent,
                group,
                info: false,
                separator: false,
                enabled,
                checked,
            });
        }
        let clicked = enabled && !group && !session.fired && session.target.as_ref() == Some(&id);
        session.fired |= clicked;
        (id, clicked)
    });
    let mut response = ui.interact(egui::Rect::NOTHING, Id::unique(("menu_command_scan", id)), egui::Sense::hover());
    if clicked {
        response.flags.insert(egui::response::Flags::FAKE_PRIMARY_CLICKED);
    }
    response
}

pub fn information(icon: &str, label: &str, separator: bool) {
    SESSION.with_borrow_mut(|session| {
        let Some(session) = session else { return };
        let parent = format!("{PREFIX}{}", session.path.join("/"));
        let parent = if session.path.is_empty() { String::new() } else { parent };
        let count = session
            .catalogue
            .entries
            .iter()
            .filter(|entry| entry.parent == parent && (entry.info || entry.separator))
            .count();
        session.catalogue.entries.push(Entry {
            button: ToolButton {
                command: format!("{parent}/__decoration-{count}"),
                icon: icon.into(),
                tooltip: aurora_i18n::key_of(label).map_or_else(|| label.to_owned(), |key| format!("@{key}")),
            },
            parent,
            group: false,
            info: !separator,
            separator,
            enabled: false,
            checked: None,
        });
    });
}

pub fn descend(ui: &mut Ui, enabled: bool, body: impl FnOnce(&mut Ui)) {
    SESSION.with_borrow_mut(|session| {
        if !enabled && let Some(session) = session {
            session.disabled += 1;
        }
    });
    body(ui);
    SESSION.with_borrow_mut(|session| {
        if !enabled && let Some(session) = session {
            session.disabled -= 1;
        }
    });
}

fn catalogue_id() -> Id {
    Id::unique("main_menu_command_catalogue")
}
fn request_id() -> Id {
    Id::unique("main_menu_command_request")
}

pub fn publish(ctx: &egui::Context, catalogue: Catalogue) {
    let catalogue = Arc::new(catalogue);
    CURRENT.with_borrow_mut(|current| *current = catalogue.clone());
    ctx.data_mut(|data| data.insert_temp(catalogue_id(), catalogue));
}

pub fn current() -> Arc<Catalogue> {
    CURRENT.with_borrow(Clone::clone)
}

pub fn catalogue(ctx: &egui::Context) -> Arc<Catalogue> {
    ctx.data(|data| data.get_temp::<Arc<Catalogue>>(catalogue_id())).unwrap_or_default()
}

pub fn request(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|data| data.insert_temp(request_id(), id.to_owned()));
}

pub fn take_request(ctx: &egui::Context) -> Option<String> {
    ctx.data_mut(|data| data.remove_temp::<String>(request_id()))
}

pub fn available(ctx: &egui::Context, command: &str) -> bool {
    catalogue(ctx).find(command).is_some_and(|entry| entry.enabled)
}

pub fn checked(ctx: &egui::Context, command: &str) -> bool {
    catalogue(ctx).find(command).and_then(|entry| entry.checked).unwrap_or(false)
}

/// The menu uses the same drag payload, appearance editor and lock as all bars.
pub fn show(ui: &mut Ui, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, parent: &str) {
    let catalogue = catalogue(ui.ctx());
    show_children(ui, p, toolbar, settings, &catalogue, parent, &mut HashSet::new());
}

fn show_children(
    ui: &mut Ui,
    p: &Palette,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    catalogue: &Catalogue,
    parent: &str,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(parent.to_owned()) {
        return;
    }
    let children = catalogue.children(parent, &settings.menu_layout);
    let mut rects = Vec::new();
    let mut ranks = Vec::new();
    for (rank, entry) in children.iter().enumerate() {
        if entry.separator {
            menu::separator(ui, p);
            continue;
        }
        if entry.info {
            menu::info(ui, p, &entry.button.icon, super::label(&entry.button));
            continue;
        }
        let button = &entry.button;
        let appearance = settings
            .appearance
            .get(&super::appearance::menu_key(&button.command))
            .cloned()
            .unwrap_or_default();
        let icon = appearance.icon.as_deref().unwrap_or(&button.icon);
        let label = super::label(button);
        let mode = appearance.mode.unwrap_or(super::ButtonMode::IconsWithText);
        let checked = if button.command == "location_bar" {
            Some(crate::ui::bars::location_popup::is_open(ui.ctx()))
        } else {
            entry.checked
        };
        let response = if entry.group {
            menu::command_submenu(ui, p, icon, label, mode, true, |ui| {
                show_children(ui, p, toolbar, settings, catalogue, &button.command, visited);
            })
            .interact(egui::Sense::click_and_drag())
        } else {
            menu::command_row(ui, p, icon, label, mode, entry.enabled, checked, !settings.locked)
        };
        if response.rect.is_positive() {
            rects.push(response.rect);
            ranks.push(rank);
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique(("main_menu_entry", &button.command)), response.rect));
        response.clone().on_hover_text(label);
        if !settings.locked && response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(Drag {
                button: button.clone(),
                from: None,
                members: Vec::new(),
            });
        }
        toolbar.menu_appearance_context(&response, p, button, settings.locked);
        if !entry.group && response.clicked() && entry.enabled && !ToolbarUi::intercepting(ui.ctx()) {
            request(ui.ctx(), &button.command);
            ui.close();
        }
    }
    if !settings.locked && ToolbarUi::dragging(ui.ctx()) {
        let end = ui.add(egui::Label::new(aurora_i18n::tr!("toolbar-drop-here")).sense(egui::Sense::hover()));
        let rect = rects.iter().copied().fold(end.rect, egui::Rect::union);
        let zone = ui.interact(rect, Id::unique(("main_menu_drop", parent)), egui::Sense::hover());
        super::drop_preview::protect(ui.ctx(), rect, ui.layer_id());
        if zone.dnd_hover_payload::<Drag>().is_some()
            && let Some(pos) = ui.ctx().pointer_interact_pos()
        {
            let rank = super::insertion(&rects, pos, true);
            let y = rects
                .get(rank)
                .map_or_else(|| rects.last().map_or(end.rect.top(), |rect| rect.bottom()), |rect| rect.top());
            ui.painter().hline(rect.x_range(), y, egui::Stroke::new(2.0, p.violet_light));
        }
        if zone.dnd_hover_payload::<Drag>().is_some()
            && let Some(drag) = zone.dnd_release_payload::<Drag>()
            && drag.members.is_empty()
            && catalogue.can_place(&drag.button.command, parent, &settings.menu_layout)
            && let Some(pos) = ui.ctx().pointer_interact_pos()
        {
            toolbar.pending = Some(Edit::MenuInsert(
                parent.to_owned(),
                drag.button.command.clone(),
                ranks.get(super::insertion(&rects, pos, true)).copied().unwrap_or(children.len()),
            ));
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("toolbar_drop_handled"), true));
        }
    }
    visited.remove(parent);
}
