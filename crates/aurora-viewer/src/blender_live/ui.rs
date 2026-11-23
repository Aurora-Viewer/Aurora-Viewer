//! The LiveMesh floater (Build › LiveMesh), in the Director's language
//! (`ui/director.rs`): text tabs, folding sections, label rows and stacked
//! cards for the objects. Aurora's own: the Legacy Viewer has no live link
//! with Blender.
//!
//! - A status line above every tab: the link, the revision, Stop.
//! - Objects: one card per object Blender sends (linksets unfold to their
//!   prims), with its visibility and a menu; the port and Remove meshes in
//!   a folded Connection section.
//! - Place: where the selected static object is, and its position,
//!   rotation and size.
//! - Avatar: the shape transfer to Blender.
//! - Import: the selected object frozen into the mesh importer.
//! - Plugins: the plug-in in each Blender found, and the manual ZIP.

use super::*;
use crate::theme::Palette;
use crate::ui::menu;
use crate::ui::widgets::{self, ActionKind, Floater};
use aurora_i18n::UiUpper;
use egui::{RichText, Sense, Stroke, Vec2};

const SIZE: Vec2 = Vec2::new(400.0, 660.0);
const MIN_SIZE: Vec2 = Vec2::new(340.0, 350.0);
/// A linked prim's card stands this far in from its root's.
const KID_INDENT: f32 = 28.0;

impl Tab {
    pub(super) const ALL: [Tab; 5] = [Tab::Objects, Tab::Place, Tab::Avatar, Tab::Import, Tab::Plugins];

    fn label(self) -> &'static str {
        match self {
            Tab::Objects => aurora_i18n::tr!("blender-live-objects"),
            Tab::Place => aurora_i18n::tr!("blender-live-tab-place"),
            Tab::Avatar => aurora_i18n::tr!("blender-live-tab-avatar"),
            Tab::Import => aurora_i18n::tr!("blender-live-upload"),
            Tab::Plugins => aurora_i18n::tr!("blender-live-plugins"),
        }
    }
}

pub(super) fn show(live: &mut Live, ctx: &egui::Context, p: &Palette, can_upload: bool) -> bool {
    // a Derez, Detach, Drop or Attach from the world's menu while the window
    // is closed or minimized keeps its Undo, over the world
    if !live.open || widgets::floater_minimized(ctx, "blender_live") {
        world_undo_toast(live, ctx, p);
    }
    if !live.open {
        return false;
    }
    let mut open = live.open;
    let mut start = false;
    let mut floater = Floater::new(
        "blender_live",
        aurora_i18n::tr!("blender-live-title"),
        egui::pos2(70.0, 110.0),
        SIZE,
    )
    .steady_size();
    floater.min_size = MIN_SIZE;
    if live.received_at.is_some() {
        // the "2 s ago" times move on
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
    floater.show(ctx, p, &mut open, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        // the body keeps the window's width: a long label or a row of fields
        // wraps instead of widening the window
        ui.set_max_width(ui.available_width());
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        let labels = Tab::ALL.map(|tab| (tab.label(), true));
        let mut index = Tab::ALL.iter().position(|tab| *tab == live.tab).unwrap_or(0);
        if widgets::wrapping_tabs(ui, p, &mut index, &labels) {
            live.tab = Tab::ALL[index.min(Tab::ALL.len() - 1)];
        }
        start = status(live, ui, p);
        undo_toast(live, ui, p);
        // each tab keeps its own scroll position
        egui::ScrollArea::vertical()
            .id_salt(("blender_live_tab", index))
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| match live.tab {
                Tab::Objects => objects(live, ui, p, can_upload),
                Tab::Place => place(live, ui, p),
                Tab::Avatar => avatar(live, ui, p),
                Tab::Import => import(live, ui, p, can_upload),
                Tab::Plugins => plugins(live, ui, p),
            });
    });
    live.open = open;
    start
}

/// The toast of the last placement change (Derez, Detach, Drop, Attach),
/// with its Undo, for `objects::Undo::SHOWN`. Placement is local to this
/// viewer, so the change is made at once and the toast offers it back
/// instead of asking first.
fn undo_toast(live: &mut Live, ui: &mut egui::Ui, p: &Palette) {
    let Some(text) = undo_text(live, ui.ctx()) else { return };
    egui::Frame::new()
        .fill(p.field)
        .stroke(Stroke::new(1.0, p.violet.gamma_multiply(0.45)))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: 10,
            right: 4,
            top: 4,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            undo_row(live, ui, p, &text);
        });
}

/// The same toast over the world, above the toolbar, when the window is
/// closed or minimized: the change came from the world's menu, so its Undo
/// stays within reach. It names the window, which is not in sight.
fn world_undo_toast(live: &mut Live, ctx: &egui::Context, p: &Palette) {
    let Some(text) = undo_text(live, ctx) else { return };
    egui::Area::new(egui::Id::unique("blender_live_world_undo"))
        .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -WORLD_TOAST_ABOVE))
        .show(ctx, |ui| {
            ui.set_max_width(WORLD_TOAST_WIDTH);
            widgets::overlay(
                ui,
                p,
                egui::Margin {
                    left: 12,
                    right: 6,
                    top: 6,
                    bottom: 6,
                },
                |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("blender-live-title").ui_upper())
                                .size(10.5)
                                .color(p.muted),
                        )
                        .selectable(false),
                    );
                    undo_row(live, ui, p, &text);
                },
            );
        });
}

/// How far above the window's bottom the world's toast sits: clear of the
/// toolbar and the chat bar.
const WORLD_TOAST_ABOVE: f32 = 84.0;
const WORLD_TOAST_WIDTH: f32 = 420.0;

/// What the toast says, while it is shown.
fn undo_text(live: &Live, ctx: &egui::Context) -> Option<String> {
    use super::objects::Action;
    let undo = live.undo.as_ref()?;
    let now = std::time::Instant::now();
    if undo.expired(now) {
        return None;
    }
    ctx.request_repaint_after(objects_undo_left(undo, now));
    let name = live
        .snapshot
        .as_ref()
        .and_then(|s| s.sources.iter().find(|s| s.id == undo.source))
        .map_or_else(String::new, |s| s.name.clone());
    Some(match undo.action {
        Action::Derez => aurora_i18n::tr!("blender-live-undo-derez", name = name.as_str()),
        Action::Detach => aurora_i18n::tr!("blender-live-undo-detach", name = name.as_str()),
        Action::Drop => aurora_i18n::tr!("blender-live-undo-drop", name = name.as_str()),
        Action::Attach(point) => {
            let point = live
                .attach_choices
                .iter()
                .find(|choice| choice.0 == point)
                .map_or("", |choice| choice.2.as_str());
            aurora_i18n::tr!("blender-live-undo-attach", name = name.as_str(), point = point)
        }
        _ => return None,
    })
}

/// The toast's text and its Undo button.
fn undo_row(live: &mut Live, ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-undo")).clicked() {
                live.undo_requested = true;
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
            });
        });
    });
}

fn objects_undo_left(undo: &super::objects::Undo, now: std::time::Instant) -> std::time::Duration {
    super::objects::Undo::SHOWN.saturating_sub(now.saturating_duration_since(undo.at))
}

/// A short muted paragraph under the status line.
fn note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.muted)).wrap());
}

/// A small upper-case heading over a list, its controls on the right.
fn list_heading(ui: &mut egui::Ui, p: &Palette, text: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
            ui.add(egui::Label::new(RichText::new(text.ui_upper()).size(10.5).color(p.muted)).truncate());
        });
    });
}

/// The status line every tab shows: the link, its revision, and the button
/// that starts or stops the preview; then the errors. True when Start was
/// clicked.
fn status(live: &mut Live, ui: &mut egui::Ui, p: &Palette) -> bool {
    let mut start = false;
    let connected = live.server.as_ref().is_some_and(|server| server.shared.status.lock().connected);
    let (label, color) = if connected {
        (aurora_i18n::tr!("blender-live-connected"), p.teal)
    } else if live.running() {
        (aurora_i18n::tr!("blender-live-waiting"), p.amber)
    } else {
        (aurora_i18n::tr!("blender-live-stopped"), p.muted)
    };
    ui.horizontal(|ui| {
        widgets::status_dot(ui, color);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if live.running() {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-stop")).clicked() {
                    live.stop();
                }
            } else if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-start")).clicked() {
                start = true;
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let mut text = egui::text::LayoutJob::default();
                text.append(label, 0.0, egui::TextFormat::simple(egui::FontId::proportional(12.5), p.ink));
                let muted = egui::TextFormat::simple(egui::FontId::proportional(11.5), p.muted);
                if let Some(revision) = live.snapshot.as_ref().map(|s| s.revision) {
                    text.append(
                        &format!("  ·  {}", aurora_i18n::tr!("blender-live-revision", revision = revision)),
                        0.0,
                        muted.clone(),
                    );
                }
                if let Some(at) = live.received_at {
                    text.append(&format!("  ·  {}", ago(at).0), 0.0, muted);
                }
                ui.add(egui::Label::new(text).truncate());
            });
        });
    });
    let error = live
        .error
        .clone()
        .or_else(|| live.server.as_ref().and_then(|s| s.shared.status.lock().error.clone()));
    if let Some(error) = error {
        ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-error", error = error));
    }
    if !live.import.message.is_empty() {
        ui.colored_label(if live.import.error { p.danger } else { p.muted }, &live.import.message);
    }
    start
}

/// How long ago `at` was, as the cards and the status line write it, and
/// whether it is fresh (under five seconds: the time shows in teal).
fn ago(at: std::time::Instant) -> (String, bool) {
    let seconds = at.elapsed().as_secs();
    let text = if seconds < 60 {
        aurora_i18n::tr!("blender-live-ago-seconds", count = seconds)
    } else if seconds < 3600 {
        aurora_i18n::tr!("blender-live-ago-minutes", count = seconds / 60)
    } else {
        aurora_i18n::tr!("blender-live-ago-hours", count = seconds / 3600)
    };
    (text, seconds < 5)
}

fn time_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), egui::Color32::PLACEHOLDER)
        .size()
        .x
}

/// A card's time since its source last changed, right-aligned.
fn ago_label(ui: &mut egui::Ui, p: &Palette, at: Option<std::time::Instant>) {
    if let Some(at) = at {
        let (text, fresh) = ago(at);
        ui.label(RichText::new(text).size(11.0).color(if fresh { p.teal } else { p.muted }))
            .on_hover_text(aurora_i18n::tr!("blender-live-changed-tip"));
    }
}

/// The name a list gives a source: a linked prim's number, a linkset's
/// prims.
fn display_name(row: &links::Row) -> String {
    let source = &row.source;
    if source.link.is_some() {
        aurora_i18n::tr!("blender-object-linked-prim", number = row.number, name = source.name.as_str())
    } else {
        source.name.clone()
    }
}

/// The triangles of a source as Blender sent them.
fn triangles(source: &protocol::PreparedSource) -> usize {
    source.scene.models.iter().map(|model| model.triangles()).sum()
}

fn objects(live: &mut Live, ui: &mut egui::Ui, p: &Palette, can_upload: bool) {
    note(ui, p, aurora_i18n::tr!("blender-live-objects-help"));
    note(ui, p, aurora_i18n::tr!("blender-live-local-only"));
    let sources = live.snapshot.as_ref().map(|s| s.sources.clone()).unwrap_or_default();
    let counts = aurora_i18n::tr!(
        "blender-live-linkset-counts",
        objects = sources
            .iter()
            .filter(|s| s.link.is_none() && live.state(s.id) == SourceState::Visible)
            .count(),
        prims = live.snapshot.as_ref().map_or(0, |s| s
            .meshes
            .iter()
            .filter(|m| live.state(m.source) == SourceState::Visible && live.state(live.linked_root(m.source)) == SourceState::Visible)
            .count()),
        triangles = live.triangles
    );
    let mut shown = None;
    list_heading(ui, p, &counts, |ui| {
        let any = |state| sources.iter().any(|s| live.state(s.id) == state);
        if ui
            .add_enabled_ui(any(SourceState::Visible), |ui| {
                widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-hide-all"))
            })
            .inner
            .clicked()
        {
            shown = Some(SourceState::Hidden);
        }
        if ui
            .add_enabled_ui(any(SourceState::Hidden), |ui| {
                widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-show-all"))
            })
            .inner
            .clicked()
        {
            shown = Some(SourceState::Visible);
        }
    });
    if let Some(state) = shown {
        // removed objects stay removed
        let kept: Vec<_> = sources
            .iter()
            .filter(|s| live.state(s.id) != SourceState::Removed)
            .map(|s| s.id)
            .collect();
        for id in kept {
            live.set_state(id, state);
        }
    }
    if sources.iter().all(|s| live.state(s.id) == SourceState::Removed) {
        note(ui, p, aurora_i18n::tr!("blender-live-empty"));
    }
    let rows = live.linkset_rows();
    let mut kids_open = false;
    for (index, row) in rows.iter().enumerate() {
        let source = &row.source;
        if live.state(source.id) == SourceState::Removed {
            // a removed root's prims still show, as their own rows
            if source.link.is_none() {
                kids_open = true;
            }
            continue;
        }
        ui.push_id(source.id, |ui| {
            if source.link.is_none() {
                let kids: Vec<_> = rows[index + 1..]
                    .iter()
                    .take_while(|kid| kid.source.link.is_some())
                    .filter(|kid| live.state(kid.source.id) != SourceState::Removed)
                    .collect();
                let tris = triangles(source) + kids.iter().map(|kid| triangles(&kid.source)).sum::<usize>();
                let changed = std::iter::once(source.id)
                    .chain(kids.iter().map(|kid| kid.source.id))
                    .filter_map(|id| live.changed.get(&id).map(|entry| entry.1))
                    .max();
                let open_id = ui.make_persistent_id(("blender_live_kids", source.id));
                kids_open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(true);
                let toggle = object_card(live, ui, p, row, tris, changed, (!kids.is_empty()).then_some(kids_open), can_upload);
                if toggle {
                    kids_open = !kids_open;
                    ui.ctx().data_mut(|d| d.insert_temp(open_id, kids_open));
                }
            } else if kids_open {
                kid_card(live, ui, p, row, can_upload);
            }
        });
    }
    quick_look(live, ui, p);
    ui.add_space(4.0);
    widgets::collapsing(
        ui,
        p,
        "blender_live_connection",
        aurora_i18n::tr!("blender-live-connection"),
        false,
        |ui| {
            widgets::form_row(ui, p, aurora_i18n::tr!("blender-live-port"), |ui| {
                ui.add_enabled(!live.running(), egui::DragValue::new(&mut live.port).range(1024..=65535));
                ui.monospace("127.0.0.1");
            });
            if ui
                .add_enabled_ui(sources.iter().any(|s| live.state(s.id) != SourceState::Removed), |ui| {
                    widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-clear"))
                })
                .inner
                .clicked()
            {
                for source in &sources {
                    live.set_state(source.id, SourceState::Removed);
                }
            }
        },
    );
}

/// A card's face: filled, and outlined in violet when it is the selected
/// object.
fn card_frame(ui: &egui::Ui, p: &Palette, selected: bool, kid: bool) -> egui::Frame {
    let fill = if selected {
        p.violet.gamma_multiply(0.16)
    } else if kid {
        p.field.gamma_multiply(0.7)
    } else {
        p.field
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(if selected { Stroke::new(1.0, p.violet) } else { Stroke::NONE })
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: 4,
            right: 6,
            top: 4,
            bottom: 4,
        })
}

/// The eye that shows or hides a source in the preview.
fn eye(live: &mut Live, ui: &mut egui::Ui, p: &Palette, id: Uuid) {
    let visible = live.state(id) == SourceState::Visible;
    if widgets::icon_button(
        ui,
        p,
        if visible { "eye-filled" } else { "eye-off" },
        aurora_i18n::tr!("blender-live-visible"),
        true,
    ) {
        live.set_state(id, if visible { SourceState::Hidden } else { SourceState::Visible });
    }
}

/// The side of a card's thumbnail, and of Quick look's view.
const THUMBNAIL: f32 = 40.0;
const QUICK_LOOK: f32 = 150.0;

/// A palette colour as the preview renderer takes it (sRGB, 0..1).
fn preview_color(color: egui::Color32) -> [f32; 4] {
    color.to_srgba_unmultiplied().map(|v| f32::from(v) / 255.0)
}

/// The camera of a thumbnail: the preview's three-quarter view, a little
/// closer so that the object fills its square.
fn thumbnail_view(p: &Palette) -> aurora_render::preview::View {
    aurora_render::preview::View {
        distance: 2.4,
        background: preview_color(p.raised),
        ambient: [0.25; 3],
        ..Default::default()
    }
}

/// A card's live thumbnail (`thumbnail.rs`), or its empty square while it
/// is built. A click opens Quick look on it, or closes it.
fn thumbnail(live: &mut Live, ui: &mut egui::Ui, p: &Palette, id: Uuid) {
    let size = Vec2::splat(THUMBNAIL);
    let tip = aurora_i18n::tr!("blender-live-quick-look-tip");
    let response = match live.thumbnails.get(id) {
        Some(model) => {
            aurora_render::preview::paint_hint(ui, egui::Id::unique(("blender_live_thumbnail", id)), model, thumbnail_view(p), size)
        }
        None => {
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
            response
        }
    };
    if live.thumbnails.updating(id) {
        sweep(ui, p, response.rect, live.still);
    }
    if response.on_hover_text(tip).clicked() {
        live.quick_look = if live.quick_look == Some(id) { None } else { Some(id) };
        live.quick_view = aurora_render::preview::View {
            distance: 2.6,
            ..thumbnail_view(p)
        };
    }
}

/// How long the sweep takes to cross a thumbnail.
const SWEEP: f64 = 1.6;

/// The light that crosses a thumbnail while its update is on its way, so a
/// change in Blender shows on its card before its new picture. With Reduce
/// animations, a still wash in its place.
fn sweep(ui: &egui::Ui, p: &Palette, rect: egui::Rect, still: bool) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let light = p.teal.gamma_multiply(0.3);
    if still {
        painter.rect_filled(rect, ui.visuals().window_corner_radius, p.teal.gamma_multiply(0.15));
        return;
    }
    let t = ui.input(|i| i.time) % SWEEP / SWEEP;
    // ease in and out, from fully left of the picture to fully right
    let eased = (t * t * (3.0 - 2.0 * t)) as f32;
    let width = rect.width();
    let x = rect.left() - width + eased * 2.0 * width;
    // a band slanted a little, clear at its edges and lit in its middle
    let slant = rect.height() * 0.18;
    let clear = egui::Color32::TRANSPARENT;
    let mut mesh = egui::Mesh::default();
    for (dx, color) in [(0.3, clear), (0.5, light), (0.7, clear)] {
        let at = x + dx * width;
        mesh.colored_vertex(egui::pos2(at + slant, rect.top()), color);
        mesh.colored_vertex(egui::pos2(at - slant, rect.bottom()), color);
    }
    for i in [0, 2] {
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    painter.add(mesh);
    ui.ctx().request_repaint();
}

/// Quick look: the selected card's object larger, turned by a drag, with
/// what the card says of it. Under the cards, until Close or a second click
/// on its thumbnail.
fn quick_look(live: &mut Live, ui: &mut egui::Ui, p: &Palette) {
    let Some(id) = live.quick_look else {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<Uuid>(egui::Id::unique("blender_live_quick_look_shown")));
        return;
    };
    let Some(source) = live
        .snapshot
        .as_ref()
        .and_then(|s| s.sources.iter().find(|s| s.id == id))
        .cloned()
        .filter(|_| live.state(id) != SourceState::Removed)
    else {
        live.quick_look = None;
        return;
    };
    let Some(model) = live.thumbnails.get(id) else { return };
    // the colours follow the theme; the camera is the user's
    live.quick_view.background = preview_color(p.raised);
    live.quick_view.ambient = [0.25; 3];
    let mut close = false;
    let response = widgets::surface(ui, p, |ui| {
        ui.horizontal_top(|ui| {
            aurora_render::preview::paint_with_pose(
                ui,
                egui::Id::unique(("blender_live_quick_look", id)),
                model,
                &mut live.quick_view,
                Vec2::splat(QUICK_LOOK),
                live.quick_pose.1.clone(),
            )
            .on_hover_text(aurora_i18n::tr!("blender-live-quick-look-drag"));
            ui.vertical(|ui| {
                ui.label(RichText::new(&source.name).strong().size(13.0).color(p.ink));
                // the card's line: the root with its prims
                let rows = live.linkset_rows();
                let row = rows.iter().find(|row| row.source.id == id);
                let prims = row.map_or(1, |row| row.prims);
                let tris = triangles(&source)
                    + live.snapshot.as_ref().map_or(0, |s| {
                        s.sources
                            .iter()
                            .filter(|kid| kid.link.is_some_and(|link| link.parent == id) && live.state(kid.id) != SourceState::Removed)
                            .map(triangles)
                            .sum()
                    });
                let detail = if source.rigged() {
                    aurora_i18n::tr!("blender-live-card-rigged", triangles = tris)
                } else if prims > 1 {
                    aurora_i18n::tr!("blender-live-card-linkset", prims = prims, triangles = tris)
                } else {
                    aurora_i18n::tr!("blender-live-card-static", triangles = tris)
                };
                ui.label(RichText::new(detail).size(11.5).color(p.muted));
                if let Some((_, at)) = live.changed.get(&id) {
                    note(ui, p, &aurora_i18n::tr!("blender-live-quick-look-changed", ago = ago(*at).0));
                }
                if source.rigged() && live.quick_pose.1.is_some() {
                    let bend = aurora_i18n::tr!(
                        "blender-live-quick-look-bend",
                        elbows = live.bend.elbows.round() as i64,
                        knees = live.bend.knees.round() as i64
                    );
                    ui.label(RichText::new(bend).size(11.5).color(p.teal));
                }
                note(ui, p, aurora_i18n::tr!("blender-live-quick-look-drag"));
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-close")).clicked() {
                    close = true;
                }
            });
        })
        .response
    });
    if ui
        .ctx()
        .data(|d| d.get_temp::<Uuid>(egui::Id::unique("blender_live_quick_look_shown")))
        != Some(id)
    {
        // just opened: bring it into view
        response.scroll_to_me(None);
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique("blender_live_quick_look_shown"), id));
    }
    if close {
        live.quick_look = None;
        ui.ctx()
            .data_mut(|d| d.remove_temp::<Uuid>(egui::Id::unique("blender_live_quick_look_shown")));
    }
}

/// The `…` menu of a card: import it, place it.
fn card_menu(live: &mut Live, ui: &mut egui::Ui, p: &Palette, source: &protocol::PreparedSource, can_upload: bool) {
    widgets::icon_menu(ui, p, "dots-filled", aurora_i18n::tr!("blender-live-more"), |ui| {
        if menu::item_if(ui, p, "file-upload-filled", aurora_i18n::tr!("blender-live-upload"), can_upload) {
            live.selected = Some(source.id);
            live.upload_requested = Some(source.id);
        }
        if !source.rigged() && menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("blender-live-place")) {
            live.selected = Some(source.id);
            live.tab = Tab::Place;
        }
    });
}

/// The name and the line under it, clickable: selects the source.
fn name_block(
    live: &mut Live,
    ui: &mut egui::Ui,
    p: &Palette,
    source: &protocol::PreparedSource,
    name: &str,
    detail: Option<(&str, bool)>,
    width: f32,
) {
    let visible = live.state(source.id) == SourceState::Visible;
    let name_font = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let detail_font = egui::FontId::proportional(10.5);
    let name_h = ui.fonts_mut(|f| f.row_height(&name_font));
    let detail_h = if detail.is_some() {
        ui.fonts_mut(|f| f.row_height(&detail_font)) + 2.0
    } else {
        0.0
    };
    let height = (name_h + detail_h).max(24.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(1.0), height), Sense::click());
    let resp = resp.on_hover_text(name);
    if resp.clicked() {
        live.selected = Some(source.id);
    }
    let painter = ui.painter_at(rect);
    let ink = if visible { p.ink } else { p.muted };
    let top = rect.center().y - (name_h + detail_h) / 2.0;
    let name_galley = ui.painter().layout(name.to_owned(), name_font, ink, f32::INFINITY);
    painter.galley(egui::pos2(rect.left(), top), name_galley, ink);
    if let Some((text, rigged)) = detail {
        let y = top + name_h + 2.0;
        let mut left = rect.left();
        if rigged {
            let tag = aurora_i18n::tr!("blender-live-tag-rigged").ui_upper();
            left += widgets::paint_row_tag(ui, p, &tag, true, left, y + detail_h / 2.0 - 1.0) + 4.0;
        }
        let galley = ui.painter().layout(text.to_owned(), detail_font, p.muted, f32::INFINITY);
        painter.galley(egui::pos2(left, y), galley, p.muted);
    }
}

/// A root object's card. `kids` is whether its linked prims are unfolded,
/// for a linkset; true when the caret was clicked.
fn object_card(
    live: &mut Live,
    ui: &mut egui::Ui,
    p: &Palette,
    row: &links::Row,
    tris: usize,
    changed: Option<std::time::Instant>,
    kids: Option<bool>,
    can_upload: bool,
) -> bool {
    let source = &row.source;
    let rigged = source.rigged();
    let detail = if rigged {
        aurora_i18n::tr!("blender-live-card-rigged", triangles = tris)
    } else if row.prims > 1 {
        aurora_i18n::tr!("blender-live-card-linkset", prims = row.prims, triangles = tris)
    } else {
        aurora_i18n::tr!("blender-live-card-static", triangles = tris)
    };
    let mut toggle = false;
    card_frame(ui, p, live.selected == Some(source.id), false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            eye(live, ui, p, source.id);
            thumbnail(live, ui, p, source.id);
            // the time, the caret and the menu take their room from the right
            let time_w = changed.map_or(0.0, |at| time_width(ui, &ago(at).0) + 6.0);
            let buttons = 20.0 + if kids.is_some() { 20.0 + 6.0 } else { 0.0 } + 6.0 + time_w;
            let width = ui.available_width() - buttons;
            name_block(live, ui, p, source, &display_name(row), Some((&detail, rigged)), width);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                card_menu(live, ui, p, source, can_upload);
                if let Some(open) = kids {
                    toggle = widgets::icon_button(
                        ui,
                        p,
                        if open { "chevron-down" } else { "chevron-right" },
                        aurora_i18n::tr!("blender-live-linked-prims"),
                        true,
                    );
                }
                ago_label(ui, p, changed);
            });
        });
    });
    toggle
}

/// A linked prim, under its root's card.
fn kid_card(live: &mut Live, ui: &mut egui::Ui, p: &Palette, row: &links::Row, can_upload: bool) {
    let source = &row.source;
    ui.horizontal(|ui| {
        ui.add_space(KID_INDENT);
        card_frame(ui, p, live.selected == Some(source.id), true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                eye(live, ui, p, source.id);
                let tris = aurora_i18n::tr!("blender-live-triangles", triangles = triangles(source));
                let tris_w = ui
                    .painter()
                    .layout_no_wrap(tris.clone(), egui::FontId::proportional(10.5), p.muted)
                    .size()
                    .x;
                let changed = live.changed.get(&source.id).map(|entry| entry.1);
                let time_w = changed.map_or(0.0, |at| time_width(ui, &ago(at).0) + 6.0);
                let width = ui.available_width() - tris_w - 20.0 - 12.0 - time_w;
                name_block(live, ui, p, source, &display_name(row), None, width);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    card_menu(live, ui, p, source, can_upload);
                    ago_label(ui, p, changed);
                    ui.label(RichText::new(tris).size(10.5).color(p.muted));
                });
            });
        });
    });
}

/// A choice among the sources of the snapshot, by the name the list gives
/// them; `filter` says which are offered.
fn source_choice(live: &mut Live, ui: &mut egui::Ui, id: &str, filter: impl Fn(&protocol::PreparedSource) -> bool) {
    let rows: Vec<_> = live
        .linkset_rows()
        .into_iter()
        .filter(|row| live.state(row.source.id) != SourceState::Removed && filter(&row.source))
        .collect();
    let shown = rows
        .iter()
        .find(|row| Some(row.source.id) == live.selected)
        .map(display_name)
        .unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(ui.available_width().min(220.0))
        .truncate()
        .show_ui(ui, |ui| {
            for row in &rows {
                ui.selectable_value(&mut live.selected, Some(row.source.id), display_name(row));
            }
        });
}

fn place(live: &mut Live, ui: &mut egui::Ui, p: &Palette) {
    use super::objects::{Action, Location};
    use glam::{EulerRot, Quat, Vec3};
    note(ui, p, aurora_i18n::tr!("blender-live-place-help"));
    widgets::form_row(ui, p, aurora_i18n::tr!("common-object"), |ui| {
        source_choice(live, ui, "blender_live_place_object", |source| !source.rigged());
    });
    let Some(source) = live
        .selected
        .and_then(|id| live.snapshot.as_ref()?.sources.iter().find(|source| source.id == id))
        .cloned()
        .filter(|source| !source.rigged())
    else {
        note(ui, p, aurora_i18n::tr!("blender-live-place-none"));
        return;
    };
    let Some(placement) = live.placements.get(&source.id).copied() else {
        note(ui, p, aurora_i18n::tr!("blender-live-place-none"));
        return;
    };
    if let Some(link) = source.link {
        let root = live
            .snapshot
            .as_ref()
            .and_then(|s| s.sources.iter().find(|s| s.id == link.parent))
            .map_or("", |s| s.name.as_str());
        note(ui, p, &aurora_i18n::tr!("blender-object-child-frame", root = root));
    }
    let location = live
        .placements
        .get(&live.linked_root(source.id))
        .map_or(placement.location, |p| p.location);
    let attached = matches!(location, Location::Attachment(_));
    let present = live.source_present(source.id);
    let status = match location {
        Location::World => aurora_i18n::tr!("blender-object-world").to_owned(),
        Location::Detached => aurora_i18n::tr!("blender-object-detached").to_owned(),
        Location::Attachment(point) => live
            .attach_choices
            .iter()
            .find(|choice| choice.0 == point)
            .map_or_else(String::new, |choice| {
                aurora_i18n::tr!("blender-object-attached", point = choice.2.as_str())
            }),
    };
    widgets::collapsing(ui, p, "blender_live_where", aurora_i18n::tr!("blender-live-where"), true, |ui| {
        ui.label(RichText::new(status).size(12.0).color(p.ink));
        ui.horizontal_wrapped(|ui| {
            for (action, label, enabled) in [
                (
                    Action::Edit,
                    aurora_i18n::tr!("blender-object-edit"),
                    present && live.root(source.id).is_some(),
                ),
                (Action::Rez, aurora_i18n::tr!("blender-object-rez"), !present),
                (Action::Drop, aurora_i18n::tr!("blender-object-drop"), present),
                (Action::Derez, aurora_i18n::tr!("blender-object-derez"), present && !attached),
                (Action::Detach, aurora_i18n::tr!("common-detach"), attached),
            ] {
                if ui.add_enabled_ui(enabled, |ui| widgets::flat_button(ui, p, label)).inner.clicked() {
                    live.object_action = Some((source.id, action));
                }
            }
            for hud in [false, true] {
                let label = if hud {
                    aurora_i18n::tr!("common-attach-to-hud")
                } else {
                    aurora_i18n::tr!("common-attach-to")
                };
                menu::button(ui, p, egui::Button::new(label), |ui| {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for (point, is_hud, name) in &live.attach_choices {
                            if *is_hud == hud && menu::item(ui, p, if hud { "device-desktop-filled" } else { "man-filled" }, name) {
                                live.object_action = Some((source.id, Action::Attach(*point)));
                            }
                        }
                    });
                });
            }
        });
    });
    widgets::collapsing(
        ui,
        p,
        "blender_live_transform",
        aurora_i18n::tr!("blender-live-transform"),
        true,
        |ui| {
            ui.add_enabled_ui(present && (source.link.is_none() || live.edit_linked), |ui| {
                let mut position = placement.position.to_array();
                let (z, y, x) = placement.rotation.to_euler(EulerRot::ZYX);
                // a level axis shows 0, never the "-0" a rounding leaves
                let mut rotation = [x, y, z].map(|r| if r.to_degrees().abs() < 5e-4 { 0.0 } else { r.to_degrees() });
                let mut size = (placement.size * placement.scale).to_array();
                let mut changed = false;
                for (label, values, speed, range) in [
                    (
                        aurora_i18n::tr!("build-ui-object-position-meters"),
                        &mut position,
                        0.01,
                        -65536.0..=65536.0,
                    ),
                    (
                        aurora_i18n::tr!("build-ui-object-rotation-degrees"),
                        &mut rotation,
                        1.0,
                        -360.0..=360.0,
                    ),
                    (aurora_i18n::tr!("build-ui-object-size-meters"), &mut size, 0.01, 0.01..=64.0),
                ] {
                    widgets::form_row(ui, p, label, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let width = ((ui.available_width() - 8.0) / 3.0).clamp(48.0, 90.0);
                        for (axis, value) in ["X ", "Y ", "Z "].into_iter().zip(values.iter_mut()) {
                            changed |= ui
                                .add_sized(
                                    Vec2::new(width, ui.spacing().interact_size.y),
                                    egui::DragValue::new(value)
                                        .prefix(axis)
                                        .speed(speed)
                                        .range(range.clone())
                                        .max_decimals(3),
                                )
                                .changed();
                        }
                    });
                }
                note(ui, p, aurora_i18n::tr!("blender-live-size-note"));
                if changed {
                    let mut position = Vec3::from_array(position);
                    if placement.parent.is_none()
                        && let Location::Attachment(point) = placement.location
                        && live.attach_choices.iter().any(|choice| choice.0 == point && !choice.1)
                    {
                        position = position.clamp_length_max(crate::build::MAX_ATTACHMENT_DIST);
                    }
                    live.object_action = Some((
                        source.id,
                        Action::Transform {
                            position,
                            rotation: Quat::from_euler(
                                EulerRot::ZYX,
                                rotation[2].to_radians(),
                                rotation[1].to_radians(),
                                rotation[0].to_radians(),
                            ),
                            scale: Vec3::from_array(size) / placement.size,
                        },
                    ));
                }
            });
        },
    );
}

fn avatar(live: &mut Live, ui: &mut egui::Ui, p: &Palette) {
    note(ui, p, aurora_i18n::tr!("blender-live-avatar-help"));
    widgets::collapsing(
        ui,
        p,
        "blender_live_bend",
        aurora_i18n::tr!("blender-live-bend-section"),
        true,
        |ui| {
            let max = crate::scene::anim::PreviewBend::MAX;
            for (label, value) in [
                (aurora_i18n::tr!("blender-live-elbows"), &mut live.bend.elbows),
                (aurora_i18n::tr!("blender-live-knees"), &mut live.bend.knees),
            ] {
                widgets::form_row(ui, p, label, |ui| {
                    ui.spacing_mut().slider_width = widgets::FORM_SLIDER_W;
                    ui.add(
                        egui::Slider::new(value, 0.0..=max)
                            .custom_parser(aurora_i18n::parse_number)
                            .fixed_decimals(0)
                            .suffix("°")
                            .trailing_fill(true),
                    );
                });
            }
            widgets::switch_labeled(ui, p, &mut live.bend.hold, aurora_i18n::tr!("blender-live-bend-hold"));
            if live.bend.active() && widgets::flat_button(ui, p, aurora_i18n::tr!("common-reset")).clicked() {
                live.bend = Default::default();
            }
            note(ui, p, aurora_i18n::tr!("blender-live-bend-note"));
        },
    );
    widgets::collapsing(
        ui,
        p,
        "blender_live_shape",
        aurora_i18n::tr!("blender-live-shape-section"),
        true,
        |ui| {
            for (action, label, tooltip, blocked, kind) in [
                (
                    shape::Action::Create,
                    aurora_i18n::tr!("blender-shape-send"),
                    aurora_i18n::tr!("blender-shape-tooltip"),
                    &live.shape.blocked,
                    ActionKind::Primary,
                ),
                (
                    shape::Action::Apply,
                    aurora_i18n::tr!("blender-shape-apply"),
                    aurora_i18n::tr!("blender-shape-apply-tooltip"),
                    &live.shape.apply_blocked,
                    ActionKind::Flat,
                ),
            ] {
                let enabled = blocked.is_none() && !live.shape.busy();
                let icon = if kind == ActionKind::Primary { "man-filled" } else { "refresh" };
                let size = Vec2::new(widgets::action_button_fit(ui, icon, label), ui.spacing().interact_size.y.max(26.0));
                if widgets::action_button_if(ui, p, icon, label, kind, size, enabled)
                    .on_hover_text(tooltip)
                    .on_disabled_hover_text(blocked.as_deref().unwrap_or(aurora_i18n::tr!("blender-shape-sending")))
                    .clicked()
                {
                    live.shape.requested = Some(action);
                }
            }
            if live.shape.busy() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(aurora_i18n::tr!("blender-shape-sending"));
                });
            }
            if !live.shape.message.is_empty() {
                ui.colored_label(if live.shape.error { p.danger } else { p.success }, &live.shape.message);
            }
        },
    );
}

fn import(live: &mut Live, ui: &mut egui::Ui, p: &Palette, can_upload: bool) {
    note(ui, p, aurora_i18n::tr!("blender-live-import-help"));
    widgets::form_row(ui, p, aurora_i18n::tr!("common-object"), |ui| {
        source_choice(live, ui, "blender_live_import_object", |_| true);
    });
    if let Some(revision) = live.revision {
        widgets::form_row(ui, p, aurora_i18n::tr!("blender-live-from"), |ui| {
            ui.label(aurora_i18n::tr!("blender-live-revision", revision = revision));
        });
    }
    let selected = live.selected.filter(|id| {
        live.snapshot
            .as_ref()
            .is_some_and(|s| s.sources.iter().any(|source| source.id == *id))
    });
    let label = aurora_i18n::tr!("blender-live-upload");
    let size = Vec2::new(
        widgets::action_button_fit(ui, "file-upload-filled", label),
        ui.spacing().interact_size.y.max(26.0),
    );
    widgets::collapsing(ui, p, "blender_live_levels", aurora_i18n::tr!("blender-live-levels"), true, |ui| {
        match live.import.levels.filter(|(source, _)| Some(*source) == selected) {
            Some((_, levels)) => {
                let high = levels[0].max(1) as f32;
                for (name, count) in [
                    aurora_i18n::tr!("import-lod-high"),
                    aurora_i18n::tr!("import-lod-medium"),
                    aurora_i18n::tr!("import-lod-low"),
                    aurora_i18n::tr!("import-lod-lowest"),
                ]
                .into_iter()
                .zip(levels)
                {
                    widgets::form_row(ui, p, name, |ui| {
                        widgets::level(ui, p, name, count as f32 / high, widgets::FORM_SLIDER_W, p.violet);
                        ui.label(RichText::new(aurora_i18n::tr!("blender-live-triangles", triangles = count)).size(11.5));
                    });
                }
            }
            None => note(ui, p, aurora_i18n::tr!("blender-live-levels-pending")),
        }
        note(ui, p, aurora_i18n::tr!("blender-live-levels-note"));
    });
    if widgets::action_button_if(
        ui,
        p,
        "file-upload-filled",
        label,
        ActionKind::Primary,
        size,
        can_upload && selected.is_some(),
    )
    .clicked()
    {
        live.upload_requested = selected;
    }
    note(ui, p, aurora_i18n::tr!("blender-live-upload-materials"));
}

fn plugins(live: &mut Live, ui: &mut egui::Ui, p: &Palette) {
    use super::installer::{Action, Phase};
    note(ui, p, aurora_i18n::tr!("blender-live-plugins-help"));
    let busy = live.installer.busy;
    let mut scan = false;
    list_heading(
        ui,
        p,
        &aurora_i18n::tr!("blender-live-bundled-version", version = super::addon::version()),
        |ui| {
            scan = ui
                .add_enabled_ui(!busy, |ui| widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-detect")))
                .inner
                .clicked();
        },
    );
    live.installer_scan |= scan;
    let installer = &live.installer;
    if installer.busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak(match installer.phase() {
                Phase::Choosing => aurora_i18n::tr!("blender-live-choosing"),
                Phase::Scanning => aurora_i18n::tr!("blender-live-detecting"),
                Phase::Checking => aurora_i18n::tr!("blender-live-checking"),
                Phase::Installing => aurora_i18n::tr!("blender-live-installing"),
            });
        });
    } else if installer.scanned && installer.installations.is_empty() {
        note(ui, p, aurora_i18n::tr!("blender-live-no-installations"));
    }
    if let Some(report) = &installer.success {
        let [major, minor, patch] = report.version;
        ui.colored_label(
            p.success,
            aurora_i18n::tr!("blender-live-install-success", version = format!("{major}.{minor}.{patch}")),
        );
        if let Some(backup) = &report.backup {
            ui.label(aurora_i18n::tr!("blender-live-backup", path = backup.display().to_string()));
        }
        ui.label(aurora_i18n::tr!(
            "blender-live-install-location",
            path = report.target.display().to_string()
        ))
        .on_hover_text(report.profile.display().to_string());
    }
    if let Some(error) = &installer.error {
        ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-installer-error", error = error.as_str()));
    }
    let mut target = None;
    for installation in &installer.installations {
        ui.push_id(&installation.executable, |ui| {
            card_frame(ui, p, false, false).inner_margin(6).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let action = installation.action();
                let label = match action {
                    Action::Install => aurora_i18n::tr!("blender-live-install"),
                    Action::Update => aurora_i18n::tr!("blender-live-update"),
                    Action::Enable => aurora_i18n::tr!("blender-live-enable-addon"),
                    Action::Reinstall => aurora_i18n::tr!("blender-live-reinstall"),
                    Action::Newer => aurora_i18n::tr!("blender-live-newer"),
                    Action::Unsupported => aurora_i18n::tr!("blender-live-unsupported-version"),
                    Action::Failed => aurora_i18n::tr!("blender-live-detection-failed"),
                };
                let actionable = matches!(action, Action::Install | Action::Update | Action::Enable | Action::Reinstall);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let report = &installation.report;
                        if actionable {
                            if ui
                                .add_enabled_ui(!installer.busy, |ui| widgets::flat_button(ui, p, label))
                                .inner
                                .on_hover_text(report.error.as_deref().unwrap_or(label))
                                .clicked()
                            {
                                target = Some(installation.executable.clone());
                            }
                        } else {
                            ui.label(RichText::new(label).size(11.5).color(p.muted))
                                .on_hover_text(report.error.as_deref().unwrap_or(label));
                        }
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(egui::Label::new(RichText::new(installation.label()).strong().size(12.5).color(p.ink)).truncate())
                                .on_hover_text(installation.path_label());
                            let status = if let Some(version) = &report.plugin_version {
                                aurora_i18n::tr!("blender-live-installed-version", version = version.as_str())
                            } else {
                                aurora_i18n::tr!("blender-live-not-installed").to_owned()
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{} · {status}", installation.path_label()))
                                        .size(10.5)
                                        .color(p.muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(installation.path_label());
                        });
                    });
                });
            });
        });
    }
    if target.is_some() {
        live.installer_target = target;
    }
    if ui
        .add_enabled_ui(!live.installer.busy, |ui| {
            widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-choose-blender"))
        })
        .inner
        .clicked()
    {
        live.installer_choose = true;
    }
    widgets::collapsing(
        ui,
        p,
        "blender_live_manual_zip",
        aurora_i18n::tr!("blender-live-manual-install"),
        false,
        |ui| {
            note(ui, p, aurora_i18n::tr!("blender-live-instructions"));
            ui.add_enabled_ui(!live.addon.busy, |ui| {
                widgets::radio_value(
                    ui,
                    p,
                    &mut live.addon.legacy,
                    false,
                    aurora_i18n::tr!("blender-live-package-extension"),
                );
                widgets::radio_value(ui, p, &mut live.addon.legacy, true, aurora_i18n::tr!("blender-live-package-legacy"));
            });
            if ui
                .add_enabled_ui(!live.addon.busy, |ui| {
                    widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-save-addon"))
                })
                .inner
                .clicked()
            {
                live.save_addon = true;
            }
            if live.addon.saved {
                ui.colored_label(p.violet_light, aurora_i18n::tr!("blender-live-addon-saved"));
            }
            if let Some(error) = &live.addon.error {
                ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-addon-error", error = error.as_str()));
            }
        },
    );
}
