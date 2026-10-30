//! The Dev Toolbox window (dev_toolbox/): the target bar, the tools in
//! groups the user rearranges (Customize: drag a tool within or between
//! groups, a group by its title), "Send a diagnostic" with what the chosen
//! tool copies, and the last warnings at the bottom. The app runs the tools
//! (app/dev_toolbox.rs) and gives the window what it shows (`View`).

use crate::dev_toolbox::Target;
use crate::dev_toolbox::tools::{self, Group, Kind, Layout};
use crate::theme::Palette;
use crate::ui::{menu, widgets};
use aurora_i18n::tr;
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const WINDOW: &str = "dev_toolbox";

/// Columns of the tool grid.
const COLUMNS: usize = 4;
const TOOL_H: f32 = 48.0;
const GAP: f32 = 6.0;
/// Width of "Send a diagnostic".
const SEND_W: f32 = 420.0;
/// The Copy button's narrowest beside Save and Log; narrower, they go on a
/// row of their own.
const COPY_MIN_W: f32 = 120.0;
/// Room around a button's content (`widgets::action_button_keyed_content`).
const KEYED_PAD: f32 = 24.0;
/// "Copied" stays on the button this long.
const COPIED_FOR: Duration = Duration::from_secs(2);
/// The window's default place: under the top bar, above the bottom one.
const TOP: f32 = 56.0;
const BOTTOM: f32 = 64.0;
const SIDE: f32 = 20.0;
/// Under this height, "Send a diagnostic" gives its options' hints on
/// hover, so that the preview and its buttons keep their room.
const SEND_TIGHT_H: f32 = 460.0;

/// What the app gives the window each frame.
#[derive(Default)]
pub struct View {
    /// The toggles that are on.
    pub on: HashSet<&'static str>,
    /// The second line of a button with something live ("8.4 ms · worst 31").
    pub live: HashMap<&'static str, String>,
    /// What the target bar names, and its hint.
    pub target_text: String,
    pub shortcut: Option<crate::keybinds::Binding>,
    /// The last warnings: (clock, error, text).
    pub warnings: Vec<(String, bool, String)>,
    /// "Send a diagnostic": the full text that Copy copies (header, system
    /// configuration and anonymisation applied); None before the tool ran.
    pub preview: Option<String>,
    /// The tool waits for the names of the worn items (J1, J2).
    pub waiting: bool,
    /// The surface under the cursor is frozen (Aim).
    pub frozen: bool,
    /// Aim waits for a click on a surface of the world.
    pub picking: bool,
}

/// What the window asks the app to do.
#[derive(Debug, Clone)]
pub enum Action {
    /// A tool was pressed.
    Press(&'static str),
    Copy,
    Save,
    Log,
    /// Aim: wait for a click on a surface of the world (the target then
    /// stays), or follow the pointer again.
    Aim,
    /// One of Avatar health's actions (O8).
    Health(Health),
}

/// O8: the usual fixes of a broken avatar (the Me menu's Avatar health).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    StopAnimations,
    Undeform,
    Rebake,
    RefreshAttachments,
    ResetSkeleton,
}

/// The window's own state.
pub struct DevToolboxUi {
    pub edit: bool,
    pub query: String,
    pub target: Target,
    /// "Hide residents' names" (on by default: copies end up in public issues).
    pub hide_names: bool,
    /// "Attach the system configuration (J4)".
    pub with_system: bool,
    /// The tool "Send a diagnostic" shows.
    pub selected: &'static str,
    pub copied_at: Option<Instant>,
    pending: Option<Edit>,
    actions: Vec<Action>,
}

impl Default for DevToolboxUi {
    fn default() -> Self {
        DevToolboxUi {
            edit: false,
            query: String::new(),
            target: Target::Nearest,
            hide_names: true,
            with_system: false,
            selected: "J2",
            copied_at: None,
            pending: None,
            actions: Vec::new(),
        }
    }
}

/// A change of the layout, applied after the frame's drawing.
enum Edit {
    Move((usize, usize), (usize, usize)),
    MoveGroup(usize, usize),
    Remove(usize, usize),
    Add(usize, &'static str),
    Favorite(&'static str),
    NewGroup,
    Reset,
}

/// A tool being dragged: its group and rank.
#[derive(Clone, Copy)]
struct DragTool(usize, usize);
/// A group being dragged by its title.
#[derive(Clone, Copy)]
struct DragGroup(usize);

/// The tools a copy or a live view stands for, and what "Send a
/// diagnostic" says they hold.
fn what(id: &str) -> &'static str {
    match id {
        "J1" => tr!("dev-toolbox-what-j1"),
        "J2" => tr!("dev-toolbox-what-j2"),
        "J3" => tr!("dev-toolbox-what-j3"),
        "J4" => tr!("dev-toolbox-what-j4"),
        "J5" => tr!("dev-toolbox-what-j5"),
        "J8" => tr!("dev-toolbox-what-j8"),
        "J9" => tr!("dev-toolbox-what-j9"),
        "L1" => tr!("dev-toolbox-what-l1"),
        "L2" => tr!("dev-toolbox-what-l2"),
        "L3" => tr!("dev-toolbox-what-l3"),
        "N2" => tr!("dev-toolbox-what-n2"),
        "N3" => tr!("dev-toolbox-what-n3"),
        _ => "",
    }
}

/// Whether pressing the tool shows it in "Send a diagnostic".
pub fn shows_result(id: &str) -> bool {
    tools::find(id).is_some_and(|t| matches!(t.kind, Kind::Copy | Kind::Live)) && !matches!(id, "J6" | "J7")
}

/// The window. Returns what the app must do, and whether the layout
/// changed (to save).
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    st: &mut DevToolboxUi,
    layout: &mut Layout,
    view: &View,
    open: &mut bool,
) -> (Vec<Action>, bool) {
    // between the top bar and the bottom one, whatever the screen
    let screen = ctx.content_rect();
    let room = Vec2::new(screen.width() - 2.0 * SIDE, screen.height() - TOP - BOTTOM);
    let size = Vec2::new(1440.0, 900.0).min(room);
    let pos = Pos2::new(screen.center().x - size.x / 2.0, screen.top() + TOP);
    let mut floater = widgets::Floater::new(WINDOW, tr!("dev-toolbox-title"), pos, size)
        .help(tr!("dev-toolbox-help"))
        .steady_size();
    floater.min_size = Vec2::new(760.0, 480.0).min(room);
    floater.show(ctx, p, open, |ui| body(ui, p, st, layout, view));
    let changed = match st.pending.take() {
        Some(edit) => {
            let before = layout.clone();
            match edit {
                Edit::Move(from, to) => layout.move_tool(from, to),
                Edit::MoveGroup(from, to) => layout.move_group(from, to),
                Edit::Remove(g, i) => layout.remove_tool(g, i),
                Edit::Add(g, id) => layout.add_tool(g, id),
                Edit::Favorite(id) => layout.toggle_favorite(id),
                Edit::NewGroup => layout.add_group(tr!("dev-toolbox-new-group-title")),
                Edit::Reset => *layout = Layout::default_layout(),
            }
            *layout != before
        }
        None => false,
    };
    (std::mem::take(&mut st.actions), changed)
}

fn body(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, layout: &Layout, view: &View) {
    header_row(ui, p, st, view);
    ui.add_space(6.0);
    target_row(ui, p, st, view);
    if st.edit {
        ui.add_space(4.0);
        edit_row(ui, p, st);
    }
    ui.add_space(6.0);
    let bottom = 60.0;
    let h = (ui.available_height() - bottom).max(200.0);
    let send_w = SEND_W.min(ui.available_width() * 0.45);
    ui.horizontal_top(|ui| {
        let left_w = ui.available_width() - send_w - 10.0;
        let column = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(Vec2::new(left_w, h), column, |ui| {
            ui.set_min_size(Vec2::new(left_w, h));
            ui.set_max_width(left_w);
            egui::ScrollArea::vertical()
                .id_salt("dev_toolbox_tools")
                .auto_shrink([false, false])
                .max_height(h)
                .show(ui, |ui| groups(ui, p, st, layout, view));
        });
        ui.add_space(4.0);
        ui.allocate_ui_with_layout(Vec2::new(send_w, h), column, |ui| {
            ui.set_min_size(Vec2::new(send_w, h));
            ui.set_max_width(send_w);
            widgets::surface(ui, p, |ui| send_panel(ui, p, st, view));
        });
    });
    ui.add_space(4.0);
    live_strip(ui, p, st, view);
}

/// The shortcut, how many toggles are on, the search field and Customize.
fn header_row(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, view: &View) {
    ui.horizontal(|ui| {
        if let Some(binding) = &view.shortcut {
            let caps = widgets::keycap::caps(ui, binding, p.panel);
            let w = widgets::keycap::row_width(&caps);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::hover());
            widgets::keycap::paint_row(ui, p, &caps, rect.left_center(), p.panel);
        }
        let n = view.on.len();
        ui.label(RichText::new(tr!("dev-toolbox-toggles-on", count = n)).size(12.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (icon, label, kind) = if st.edit {
                ("check", tr!("dev-toolbox-done"), widgets::ActionKind::Primary)
            } else {
                ("pencil-filled", tr!("dev-toolbox-customize"), widgets::ActionKind::Flat)
            };
            if widgets::action_button(ui, p, icon, label, kind, Vec2::new(0.0, 26.0)).clicked() {
                st.edit = !st.edit;
            }
            widgets::search_box(ui, p, &mut st.query, tr!("dev-toolbox-search"), 250.0);
        });
    });
}

/// "Target": nearest avatar, selection or under the cursor.
fn target_row(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, view: &View) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("dev-toolbox-target")).size(12.0).color(p.muted));
        let labels = [
            tr!("dev-toolbox-target-nearest"),
            tr!("dev-toolbox-target-selection"),
            tr!("dev-toolbox-target-cursor"),
        ];
        let mut i = Target::ALL.iter().position(|t| *t == st.target).unwrap_or(0);
        let options: Vec<(&str, bool)> = labels.iter().map(|l| (*l, true)).collect();
        if widgets::segmented(ui, p, &mut i, &options) {
            st.target = Target::ALL[i];
        }
        let hint = match st.target {
            Target::Nearest => tr!("dev-toolbox-target-nearest-hint"),
            Target::Selection => tr!("dev-toolbox-target-selection-hint"),
            Target::UnderCursor => tr!("dev-toolbox-target-cursor-hint"),
        };
        let (label, kind) = if view.frozen {
            (tr!("dev-toolbox-aim-frozen"), widgets::ActionKind::Primary)
        } else if view.picking {
            (tr!("dev-toolbox-aim-picking"), widgets::ActionKind::Primary)
        } else {
            (tr!("dev-toolbox-aim"), widgets::ActionKind::Flat)
        };
        let width = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), FontId::proportional(12.0), p.ink)
                .size()
                .x
        };
        let aim_w = width(label) + 48.0;
        let hint_w = width(hint);
        let spacing = ui.spacing().item_spacing.x;
        let text_w = (ui.available_width() - aim_w - hint_w - 3.0 * spacing).max(40.0);
        ui.allocate_ui(Vec2::new(text_w, 20.0), |ui| {
            ui.set_width(text_w);
            ui.add(egui::Label::new(RichText::new(&view.target_text).size(12.0).color(p.ink)).truncate());
        });
        ui.label(RichText::new(hint).size(12.0).color(p.muted));
        if widgets::action_button(ui, p, "crosshair", label, kind, Vec2::new(0.0, 26.0))
            .on_hover_text(tr!("dev-toolbox-aim-help"))
            .clicked()
        {
            st.target = Target::UnderCursor;
            st.actions.push(Action::Aim);
        }
    });
}

/// Customize: what to do, a new group, the default layout.
fn edit_row(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi) {
    ui.horizontal(|ui| {
        widgets::glyph(ui, "grip-vertical", p.violet_light, 14.0);
        ui.add(egui::Label::new(RichText::new(tr!("dev-toolbox-edit-hint")).size(12.0).color(p.muted)).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::flat_button(ui, p, tr!("dev-toolbox-default-layout")).clicked() {
                st.pending = Some(Edit::Reset);
            }
            if widgets::flat_button(ui, p, tr!("dev-toolbox-new-group")).clicked() {
                st.pending = Some(Edit::NewGroup);
            }
        });
    });
}

/// The groups and their tools.
fn groups(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, layout: &Layout, view: &View) {
    let width = ui.available_width();
    let tool_w = ((width - GAP * (COLUMNS - 1) as f32) / COLUMNS as f32).floor();
    let mut shown_any = false;
    for (gi, g) in layout.groups.iter().enumerate() {
        let favorites = g.key == Group::Favorites.key();
        let (title, hint) = match Group::from_key(&g.key) {
            Some(b) => tools::group_title(b),
            None => (g.title.as_str(), ""),
        };
        let ranks: Vec<usize> = (0..g.tools.len())
            .filter(|&i| tools::matches(&g.tools[i], tools::label(&g.tools[i]), &st.query))
            .collect();
        if ranks.is_empty() && !(st.edit && st.query.trim().is_empty()) {
            continue;
        }
        shown_any = true;
        // the title: a group is dragged by it in Customize
        let title_resp = ui
            .horizontal(|ui| {
                if st.edit && !favorites {
                    widgets::glyph(ui, "grip-vertical", p.muted_dim, 12.0);
                }
                ui.label(RichText::new(title).size(12.0).strong().color(p.ink));
                if !hint.is_empty() {
                    ui.label(RichText::new(hint).size(12.0).color(p.muted));
                }
                ui.allocate_space(Vec2::new(ui.available_width(), 0.0));
            })
            .response;
        if st.edit {
            let title_resp = ui.interact(title_resp.rect, ui.scope_id().with(("dev_toolbox_group", gi)), Sense::drag());
            if !favorites && title_resp.drag_started_by(egui::PointerButton::Primary) {
                title_resp.dnd_set_drag_payload(DragGroup(gi));
            }
            if gi > 0 && title_resp.dnd_hover_payload::<DragGroup>().is_some() {
                let r = title_resp.rect;
                ui.painter()
                    .line_segment([r.left_top(), r.right_top()], Stroke::new(2.0, p.violet_light));
                if let Some(drag) = title_resp.dnd_release_payload::<DragGroup>() {
                    st.pending = Some(Edit::MoveGroup(drag.0, gi));
                }
            }
        }
        ui.add_space(2.0);
        // the grid
        let mut rects: Vec<(usize, Rect)> = Vec::new();
        let slots = ranks.len() + usize::from(st.edit && !favorites && st.query.trim().is_empty());
        let rows = slots.div_ceil(COLUMNS).max(1);
        let grid_h = rows as f32 * TOOL_H + (rows - 1) as f32 * GAP;
        let (grid, _) = ui.allocate_exact_size(Vec2::new(width, grid_h), Sense::hover());
        for (n, &rank) in ranks.iter().enumerate() {
            let rect = slot_rect(grid, n, tool_w);
            let id: &'static str = tools::find(&g.tools[rank]).map_or("?", |t| t.id);
            rects.push((rank, rect));
            tool_button(ui, p, st, view, layout, rect, gi, rank, id, favorites);
        }
        if st.edit && !favorites && st.query.trim().is_empty() {
            let rect = slot_rect(grid, ranks.len(), tool_w);
            add_tile(ui, p, st, layout, rect, gi);
        }
        if st.edit {
            drop_zone(ui, p, st, grid, gi, g.tools.len(), &rects);
        }
        ui.add_space(10.0);
    }
    if !shown_any {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(tr!("dev-toolbox-no-match", query = st.query.trim().to_string()))
                    .size(12.0)
                    .color(p.muted),
            );
        });
    }
}

fn slot_rect(grid: Rect, n: usize, tool_w: f32) -> Rect {
    let (row, col) = (n / COLUMNS, n % COLUMNS);
    Rect::from_min_size(
        Pos2::new(grid.left() + col as f32 * (tool_w + GAP), grid.top() + row as f32 * (TOOL_H + GAP)),
        Vec2::new(tool_w, TOOL_H),
    )
}

/// One tool: icon, label, its live line or kind, its identifier.
#[allow(clippy::too_many_arguments)]
fn tool_button(
    ui: &mut egui::Ui,
    p: &Palette,
    st: &mut DevToolboxUi,
    view: &View,
    layout: &Layout,
    rect: Rect,
    gi: usize,
    rank: usize,
    id: &'static str,
    favorites: bool,
) {
    let Some(tool) = tools::find(id) else { return };
    let sense = if st.edit { Sense::click_and_drag() } else { Sense::click() };
    let resp = ui.interact(rect, ui.scope_id().with(("dev_toolbox_tool", gi, rank)), sense);
    let on = tool.kind == Kind::Toggle && view.on.contains(id);
    let selected = shows_result(id) && st.selected == id;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if on {
        p.violet
    } else if resp.hovered() {
        p.raised
    } else {
        p.field
    };
    let painter = ui.painter();
    painter.rect_filled(rect, radius, face);
    if selected {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, p.violet_light), StrokeKind::Inside);
    } else if st.edit {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, p.muted_dim), StrokeKind::Inside);
    }
    let (ink, sub_ink, icon_ink) = if on {
        (p.on_accent, p.on_accent.gamma_multiply(0.85), p.on_accent)
    } else {
        let icon = if tool.kind == Kind::Live { p.violet_light } else { p.muted };
        (
            p.ink,
            if tool.kind == Kind::Live {
                p.ink.gamma_multiply(0.8)
            } else {
                p.muted
            },
            icon,
        )
    };
    let mut x = rect.left() + 8.0;
    if st.edit {
        widgets::paint_glyph(painter, "grip-vertical", Pos2::new(x + 5.0, rect.center().y), 12.0, p.muted_dim);
        x += 14.0;
    }
    widgets::paint_glyph(painter, tool.icon, Pos2::new(x + 8.0, rect.center().y), 16.0, icon_ink);
    x += 24.0;
    // the identifier, in a small frame at the right
    let code = painter.layout_no_wrap(id.to_owned(), FontId::proportional(10.0), if on { p.on_accent } else { p.muted });
    let code_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 8.0 - code.size().x / 2.0 - 4.0, rect.center().y),
        code.size() + Vec2::new(8.0, 2.0),
    );
    painter.rect_stroke(
        code_rect,
        radius,
        Stroke::new(1.0, if on { p.on_accent.gamma_multiply(0.4) } else { p.raised }),
        StrokeKind::Inside,
    );
    painter.galley(code_rect.center() - code.size() / 2.0, code, Color32::PLACEHOLDER);
    let text_w = (code_rect.left() - 6.0 - x).max(10.0);
    let label = tools::label(id);
    let sub = match (on, view.live.get(id)) {
        (true, Some(live)) => format!("{} · {live}", tr!("dev-toolbox-on")),
        (true, None) => tr!("dev-toolbox-on").to_string(),
        (false, Some(live)) => live.clone(),
        (false, None) => tools::kind_label(tool.kind).to_string(),
    };
    let line = |text: &str, size: f32, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(size), color);
        job.wrap = egui::text::TextWrapping {
            max_width: text_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        painter.layout_job(job)
    };
    let top = line(label, 12.0, ink);
    let bottom = line(&sub, 11.0, sub_ink);
    let total = top.size().y + bottom.size().y;
    let y = rect.center().y - total / 2.0;
    painter.galley(Pos2::new(x, y), top.clone(), Color32::PLACEHOLDER);
    painter.galley(Pos2::new(x, y + top.size().y), bottom, Color32::PLACEHOLDER);
    let resp = resp.on_hover_text(format!("{id} · {label}"));
    if st.edit {
        if resp.drag_started_by(egui::PointerButton::Primary) {
            resp.dnd_set_drag_payload(DragTool(gi, rank));
        }
    } else if resp.clicked() {
        if shows_result(id) {
            st.selected = id;
            st.copied_at = None;
        }
        if id != "O8" {
            st.actions.push(Action::Press(id));
        }
    }
    // O8: Avatar health's actions in a menu under the button
    if id == "O8" && !st.edit {
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| health_menu(ui, p, st));
    }
    // its own id: O8's dropdown is keyed on the button too
    menu::context_menu_of_button(&resp, p, |ui| {
        let pinned = layout.is_favorite(id);
        let pin = if pinned { tr!("dev-toolbox-unpin") } else { tr!("dev-toolbox-pin") };
        if menu::item(ui, p, if pinned { "pinned-filled" } else { "pin-filled" }, pin) {
            st.pending = Some(Edit::Favorite(id));
        }
        if menu::item(ui, p, "copy-filled", &tr!("dev-toolbox-copy-id", id = id.to_string())) {
            ui.ctx().copy_text(id.to_owned());
        }
        if !favorites && menu::item(ui, p, "trash-filled", tr!("dev-toolbox-remove")) {
            st.pending = Some(Edit::Remove(gi, rank));
        }
    });
}

/// O8: the usual fixes of a broken avatar (the Me menu's Avatar health).
fn health_menu(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi) {
    let rows: [(&str, &str, Health); 5] = [
        ("player-stop-filled", tr!("avatar-health-stop-animations"), Health::StopAnimations),
        ("stretching", tr!("avatar-health-undeform"), Health::Undeform),
        ("reload", tr!("ui-context-rebake"), Health::Rebake),
        ("refresh", tr!("avatar-health-refresh-attachments"), Health::RefreshAttachments),
        ("bone-filled", tr!("ui-context-reset-skeleton"), Health::ResetSkeleton),
    ];
    for (icon, label, action) in rows {
        if menu::item(ui, p, icon, label) {
            st.actions.push(Action::Health(action));
        }
    }
}

/// Customize: the tile that puts a tool in this group.
fn add_tile(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, layout: &Layout, rect: Rect, gi: usize) {
    let resp = ui.interact(rect, ui.scope_id().with(("dev_toolbox_add", gi)), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let color = if resp.hovered() { p.ink } else { p.muted };
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(1.0, p.muted_dim), StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        tr!("dev-toolbox-add-tool"),
        FontId::proportional(12.0),
        color,
    );
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        let here: Vec<&str> = layout
            .groups
            .get(gi)
            .map(|g| g.tools.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let removed = layout.removed();
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for tool in tools::TOOLS.iter().filter(|t| !here.contains(&t.id)) {
                let label = format!("{} · {}", tool.id, tools::label(tool.id));
                let detail = if removed.contains(&tool.id) {
                    tr!("dev-toolbox-removed")
                } else {
                    ""
                };
                if menu::item_detail(ui, p, tool.icon, &label, detail) {
                    st.pending = Some(Edit::Add(gi, tool.id));
                }
            }
        });
    });
}

/// Customize: where a dragged tool would go in this group, and its drop.
fn drop_zone(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, grid: Rect, gi: usize, len: usize, rects: &[(usize, Rect)]) {
    let zone = ui.interact(grid.expand(GAP / 2.0), ui.scope_id().with(("dev_toolbox_drop", gi)), Sense::hover());
    if zone.dnd_hover_payload::<DragTool>().is_none() {
        return;
    }
    let Some(pos) = ui.ctx().pointer_interact_pos() else { return };
    // the first tool the pointer is before, in reading order
    let before = rects
        .iter()
        .find(|(_, r)| pos.y < r.top() || (pos.y <= r.bottom() && pos.x < r.center().x))
        .copied();
    let (rank, x, r) = match before {
        Some((rank, r)) => (rank, r.left() - GAP / 2.0, r),
        None => match rects.last() {
            Some(&(_, r)) => (len, r.right() + GAP / 2.0, r),
            None => (0, grid.left() + 2.0, Rect::from_min_size(grid.left_top(), Vec2::new(0.0, TOOL_H))),
        },
    };
    ui.painter().line_segment(
        [Pos2::new(x, r.top() + 2.0), Pos2::new(x, r.bottom() - 2.0)],
        Stroke::new(2.0, p.violet_light),
    );
    if let Some(drag) = zone.dnd_release_payload::<DragTool>() {
        st.pending = Some(Edit::Move((drag.0, drag.1), (gi, rank)));
    }
}

/// "Send a diagnostic": the chosen tool, the options, the preview, Copy.
fn send_panel(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, view: &View) {
    let id = st.selected;
    ui.label(RichText::new(tr!("dev-toolbox-send")).size(11.0).color(p.muted));
    ui.horizontal(|ui| {
        ui.label(RichText::new(tools::label(id)).size(14.0).strong().color(p.ink));
        ui.label(RichText::new(id).size(10.0).color(p.muted));
    });
    let tight = ui.available_height() < SEND_TIGHT_H;
    if !tight {
        ui.add(egui::Label::new(RichText::new(what(id)).size(12.0).color(p.muted)).wrap());
    }
    ui.add_space(6.0);
    option_switch(
        ui,
        p,
        &mut st.hide_names,
        tr!("dev-toolbox-hide-names"),
        tr!("dev-toolbox-hide-names-hint"),
        tight,
    );
    option_switch(
        ui,
        p,
        &mut st.with_system,
        tr!("dev-toolbox-with-system"),
        tr!("dev-toolbox-with-system-hint"),
        tight,
    );
    ui.add_space(6.0);
    let text = view.preview.as_deref();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("dev-toolbox-preview")).size(11.0).color(p.muted));
        if let Some(text) = text {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let kb = text.len().div_ceil(1024);
                let stats = tr!("dev-toolbox-stats", lines = text.lines().count(), kb = kb);
                ui.label(RichText::new(stats).size(11.0).color(p.muted));
            });
        }
    });
    // the note, the buttons and the fit line are laid from the bottom up:
    // they always show, and the preview takes the height left above them
    let save_w = widgets::action_button_keyed_content(ui, "download-filled", tr!("dev-toolbox-save"), None) + KEYED_PAD;
    let log_w = widgets::action_button_keyed_content(ui, "file-text-filled", tr!("dev-toolbox-log"), None) + KEYED_PAD;
    let gap = ui.spacing().item_spacing.x;
    let two_rows = ui.available_width() - save_w - log_w - 2.0 * gap < COPY_MIN_W;
    let ready = text.is_some();
    let copied = st.copied_at.is_some_and(|t| t.elapsed() < COPIED_FOR);
    let copy_label = if copied {
        tr!("dev-toolbox-copied")
    } else {
        tr!("dev-toolbox-copy")
    };
    let copy_w = if two_rows {
        ui.available_width()
    } else {
        ui.available_width() - save_w - log_w - 2.0 * gap
    };
    let copy = |ui: &mut egui::Ui, st: &mut DevToolboxUi| {
        if widgets::action_button_if(
            ui,
            p,
            "copy-filled",
            copy_label,
            widgets::ActionKind::Primary,
            Vec2::new(copy_w, 30.0),
            ready,
        )
        .clicked()
        {
            st.copied_at = Some(Instant::now());
            st.actions.push(Action::Copy);
        }
    };
    let fits = text.is_none_or(crate::dev_toolbox::fits_comment);
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.label(RichText::new(tr!("dev-toolbox-nothing-sent")).size(11.0).color(p.muted_dim));
        ui.horizontal(|ui| {
            if !two_rows {
                copy(ui, st);
            }
            if widgets::action_button_if(
                ui,
                p,
                "download-filled",
                tr!("dev-toolbox-save"),
                widgets::ActionKind::Flat,
                Vec2::new(save_w, 30.0),
                ready,
            )
            .clicked()
            {
                st.actions.push(Action::Save);
            }
            if widgets::action_button_if(
                ui,
                p,
                "file-text-filled",
                tr!("dev-toolbox-log"),
                widgets::ActionKind::Flat,
                Vec2::new(log_w, 30.0),
                ready,
            )
            .on_hover_text(tr!("dev-toolbox-log-help"))
            .clicked()
            {
                st.actions.push(Action::Log);
            }
        });
        if two_rows {
            copy(ui, st);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if fits {
                widgets::glyph(ui, "check", p.muted, 12.0);
                ui.label(RichText::new(tr!("dev-toolbox-fits")).size(11.0).color(p.muted));
            } else {
                widgets::glyph(ui, "alert-triangle-filled", p.warn, 12.0);
                ui.label(RichText::new(tr!("dev-toolbox-too-long")).size(11.0).color(p.warn));
            }
        });
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            let box_h = (ui.available_height() - 16.0).max(24.0);
            egui::Frame::new()
                .fill(p.bar)
                .corner_radius(ui.visuals().widgets.inactive.corner_radius)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_min_height(box_h);
                    egui::ScrollArea::both()
                        .id_salt("dev_toolbox_preview")
                        .auto_shrink([false, false])
                        .max_height(box_h)
                        .show(ui, |ui| match text {
                            Some(text) => {
                                let mut lines = text.lines();
                                if let Some(first) = lines.next() {
                                    ui.label(RichText::new(first).font(FontId::monospace(11.0)).color(p.violet_light));
                                }
                                let rest: String = lines.take(400).collect::<Vec<_>>().join("\n");
                                ui.add(egui::Label::new(RichText::new(rest).font(FontId::monospace(11.0)).color(p.ink)).extend());
                            }
                            None => {
                                let note = if view.waiting {
                                    tr!("dev-toolbox-waiting")
                                } else {
                                    tr!("dev-toolbox-press-hint")
                                };
                                ui.label(RichText::new(note).size(12.0).color(p.muted));
                            }
                        });
                });
        });
    });
}

/// A switch with its label and, below, its hint (on hover when `tight`).
fn option_switch(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: &str, hint: &str, tight: bool) {
    ui.horizontal(|ui| {
        widgets::switch(ui, p, on).on_hover_text(hint);
        ui.vertical(|ui| {
            ui.label(RichText::new(label).size(12.0).color(p.ink)).on_hover_text(hint);
            if !tight {
                ui.add(egui::Label::new(RichText::new(hint).size(11.0).color(p.muted)).wrap());
            }
        });
    });
}

/// "Live": the two last warnings and J3.
fn live_strip(ui: &mut egui::Ui, p: &Palette, st: &mut DevToolboxUi, view: &View) {
    widgets::divider(ui, p);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("dev-toolbox-live")).size(12.0).strong().color(p.ink));
        // the button first, on the right edge: the warnings take what is left
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::flat_button(ui, p, tr!("dev-toolbox-see-warnings")).clicked() {
                st.selected = "J3";
                st.copied_at = None;
                st.actions.push(Action::Press("J3"));
            }
            let w = (ui.available_width() - ui.spacing().item_spacing.x).max(60.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.allocate_ui(Vec2::new(w, 36.0), |ui| {
                    ui.set_width(w);
                    ui.vertical(|ui| {
                        if view.warnings.is_empty() {
                            ui.label(RichText::new(tr!("dev-toolbox-no-warning")).size(12.0).color(p.muted));
                        }
                        for (clock, error, text) in view.warnings.iter().rev().take(2).rev() {
                            ui.horizontal(|ui| {
                                let mark = if *error { p.danger } else { p.warn };
                                ui.label(RichText::new("!").size(12.0).strong().color(mark));
                                ui.label(RichText::new(clock).size(12.0).color(p.muted_dim));
                                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ink)).truncate());
                            });
                        }
                    });
                });
            });
        });
    });
}
