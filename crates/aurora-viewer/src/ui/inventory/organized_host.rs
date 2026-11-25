//! Own each browsing session once. Re-docking transfers the same tab, including its history.
use super::organized::{self, ExplorerState, Glyph, RowStyle};
use super::organized_model::{Location, Preferences};
use super::organized_store::Store;
use super::{Facts, Icons, InvAction, Inventory, InventoryPreferences, InventoryUi, Palette, Uuid, widgets};
use aurora_i18n::tr;
use egui::{Context, Key, LayerId, Modifiers, Pos2, Rect, Ui, pos2, vec2};

#[derive(Default)]
pub(super) struct Host {
    pub(super) store: Store,
    root: Uuid,
    main: Pane,
    detached: Vec<Window>,
    next: u64,
    dragging: Option<TabDrag>,
    strips: Vec<Strip>,
    hovering: Option<(u64, f64)>,
}

struct Tab {
    id: u64,
    state: ExplorerState,
}
#[derive(Default)]
struct Pane {
    tabs: Vec<Tab>,
    active: usize,
    revealed: Option<u64>,
    closing: Vec<Tab>,
}
struct Window {
    id: u64,
    pane: Pane,
    pos: Pos2,
    open: bool,
}

fn new_id(next: &mut u64) -> u64 {
    *next += 1;
    *next
}
impl Pane {
    fn add(&mut self, next: &mut u64, location: Location) {
        self.add_mode(next, location, location == Location::FullTree);
    }
    fn add_mode(&mut self, next: &mut u64, location: Location, raw: bool) {
        self.active = self.tabs.len();
        self.tabs.push(Tab {
            id: new_id(next),
            state: ExplorerState::at_mode(location, raw),
        });
    }
    fn close_active(&mut self) {
        if let Some(tab) = self.tabs.get(self.active) {
            self.close(tab.id);
        }
    }
    fn close(&mut self, id: u64) {
        if let Some(tab) = self.remove(id) {
            self.closing.push(tab);
        }
    }

    fn finish_inline_creations(&mut self, inv: &Inventory, busy: bool, visible: bool, actions: &mut Vec<InvAction>) {
        for (n, tab) in self.tabs.iter_mut().enumerate() {
            if !visible || n != self.active {
                organized::folder::leave(&mut tab.state, inv, busy, actions);
            }
        }
        for mut tab in self.closing.drain(..) {
            organized::folder::leave(&mut tab.state, inv, busy, actions);
        }
    }

    fn remove(&mut self, id: u64) -> Option<Tab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let tab = self.tabs.remove(i);
        if i < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        Some(tab)
    }
    fn reorder(&mut self, tab: u64, before: Option<u64>) {
        let Some(index) = self.tabs.iter().position(|t| t.id == tab) else {
            return;
        };
        if before == Some(tab) || self.tabs.get(index + 1).map(|t| t.id) == before {
            return;
        }
        let active = self.tabs[self.active].id;
        let tab = self.remove(tab).unwrap();
        self.insert(tab, before);
        self.active = self.tabs.iter().position(|t| t.id == active).unwrap();
    }

    fn insert(&mut self, tab: Tab, before: Option<u64>) {
        self.active = before
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .unwrap_or(self.tabs.len());
        self.tabs.insert(self.active, tab);
    }
}

/// A gesture follows the tab ID even after its widget and floater change parents.
struct TabDrag {
    tab: u64,
    origin: u64,
    host: u64,
    original_next: Option<u64>,
    original_active: u64,
    original_pos: Pos2,
    window_grab: egui::Vec2,
    tab_grab: egui::Vec2,
    moving_window: bool,
}

fn window_name(host: u64) -> String {
    if host == 0 {
        "inventory".into()
    } else {
        format!("inventory-explorer-{host}")
    }
}

enum Destination {
    Pane(u64, Option<u64>),
    Window(Pos2),
}
struct Transfer {
    from: u64,
    tab: u64,
    to: Destination,
}
type Strip = (u64, LayerId, Rect, Vec<(Rect, u64)>);

struct DockPreview {
    host: u64,
    before: Option<u64>,
    tab: u64,
    location: Location,
    title: String,
}

#[derive(Default)]
struct Frame {
    transfer: Option<Transfer>,
    strips: Vec<Strip>,
    over_item_tab: Option<u64>,
}

impl Host {
    pub(super) fn has_detached(&self) -> bool {
        !self.detached.is_empty()
    }
    pub(super) fn close_detached(&mut self) {
        self.detached.clear();
        self.strips.clear();
        self.dragging = None;
    }
    fn pane(&mut self, id: u64) -> Option<&mut Pane> {
        if id == 0 {
            Some(&mut self.main)
        } else {
            self.detached.iter_mut().find(|w| w.id == id).map(|w| &mut w.pane)
        }
    }
    fn transfer(&mut self, op: Transfer) {
        if let Destination::Pane(id, before) = op.to
            && ((before == Some(op.tab) && op.from == id) || self.pane(id).is_none())
        {
            return;
        }
        let Some(tab) = self.pane(op.from).and_then(|p| p.remove(op.tab)) else {
            return;
        };
        match op.to {
            Destination::Pane(id, before) => self.pane(id).unwrap().insert(tab, before),
            Destination::Window(pos) => {
                let mut pane = Pane::default();
                pane.insert(tab, None);
                self.detached.push(Window {
                    id: new_id(&mut self.next),
                    pos,
                    pane,
                    open: true,
                });
            }
        }
        self.detached.retain(|w| !w.pane.tabs.is_empty());
    }
    pub(super) fn suspend(&mut self, ctx: &Context, inv: &Inventory, busy: bool, actions: &mut Vec<InvAction>) {
        // Roll back a provisional rearrangement exactly as Escape does. The
        // actual panes and tab histories stay available when Explorer returns.
        if self.dragging.is_some()
            || ctx
                .dragged_id()
                .and_then(|id| ctx.read_response(id))
                .is_some_and(|r| self.strips.iter().any(|(_, layer, _, _)| *layer == r.layer_id))
        {
            ctx.stop_dragging();
        }
        self.update_tab_drag(ctx, &Frame::default(), true);
        self.hovering = None;
        for (_, layer, _, _) in self.strips.drain(..) {
            ctx.data_mut(|data| data.remove::<u64>(layer.id.with("organized-keys")));
        }
        organized::drag_preview::cancel(ctx);
        for pane in std::iter::once(&mut self.main).chain(self.detached.iter_mut().map(|w| &mut w.pane)) {
            for tab in &mut pane.tabs {
                tab.state.suspend();
            }
            pane.finish_inline_creations(inv, busy, false, actions);
        }
        self.store.save();
    }

    pub(super) fn open_window(&mut self, ctx: &Context, root: Uuid) {
        self.open_at(ctx, root, Location::Home);
    }

    pub(super) fn open_at(&mut self, ctx: &Context, root: Uuid, place: Location) {
        self.enter(root);
        let mut pane = Pane::default();
        pane.add(&mut self.next, place);
        let offset = 20.0 * (self.detached.len() % 8) as f32;
        self.detached.push(Window {
            id: new_id(&mut self.next),
            pane,
            pos: ctx.content_rect().min + vec2(40.0 + offset, 70.0 + offset),
            open: true,
        });
    }
    fn enter(&mut self, root: Uuid) {
        self.store.enter(root);
        if self.root != root {
            self.root = root;
            self.main = Pane::default();
            self.close_detached();
            self.hovering = None;
        }
    }
    pub(super) fn show(
        &mut self,
        ctx: &Context,
        palette: &Palette,
        icons: &Icons,
        inv: &mut Inventory,
        avatar: &str,
        prefs: &mut InventoryPreferences,
        facts: &Facts,
        native: &mut InventoryUi,
        open: &mut bool,
        hover_images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
        actions: &mut Vec<InvAction>,
    ) {
        self.enter(inv.root);
        if !*open && !self.has_detached() {
            self.main.finish_inline_creations(inv, native.pending.is_some(), false, actions);
            self.store.save();
            return;
        }
        // Apply the cursor's current position before laying out the floating window.
        // The same frame supplies its real rectangle to hit testing and rendering.
        let cancel_drag = self.dragging.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if let Some(drag) = self.dragging.as_ref().filter(|d| d.moving_window && !cancel_drag)
            && let Some(pos) = ctx.input(tab_pointer_pos)
        {
            widgets::move_floater(ctx, &window_name(drag.host), pos - drag.window_grab);
        }
        // The previous frame's strip geometry lets both windows lay out the preview
        // in this frame, even when the moving window would cover the whole target.
        let preview = self.dragging.as_ref().filter(|d| d.moving_window && !cancel_drag).and_then(|drag| {
            let pos = ctx.input(|i| if i.focused { tab_pointer_pos(i) } else { None })?;
            let (host, before) = dock_target(ctx, &self.strips, drag, pos)?;
            let pane = if drag.host == 0 {
                &self.main
            } else {
                &self.detached.iter().find(|w| w.id == drag.host)?.pane
            };
            let tab = pane.tabs.iter().find(|t| t.id == drag.tab)?;
            Some(DockPreview {
                host,
                before,
                tab: tab.id,
                location: tab.state.location(),
                title: tab.state.title(inv, avatar, &self.store.layout),
            })
        });
        let traveling_host = self.dragging.as_ref().map(|d| d.host);
        let mut frame = Frame::default();
        let home = pos2((ctx.content_rect().right() - 830.0).max(12.0), 56.0);
        let mut draw = |id: u64, name: &str, pos, pane: &mut Pane, visible: &mut bool| {
            if !*visible || (preview.is_some() && traveling_host == Some(id)) {
                return;
            }
            if pane.tabs.is_empty() {
                pane.add(&mut self.next, Location::Home);
            }
            // The native Close Window accelerator is routed before Floater::show.
            // Close a tab here, while Close All and minimized floaters retain
            // the viewer's normal window-level behavior.
            if close_routed_tab(ctx, name, pane) && pane.tabs.is_empty() {
                *visible = false;
                return;
            }
            if id == 0 {
                let state = &mut pane.tabs[pane.active].state;
                if native.focus_search {
                    native.focus_search = false;
                    state.focus_search();
                }
                if let Some(id) = native.reveal
                    && state.reveal(inv, &self.store.layout, id)
                {
                    native.reveal = None;
                    native.reveal_path.clear();
                }
            }
            let mut close = false;
            widgets::Floater::new(name, tr!("common-inventory"), pos, vec2(790.0, 542.0))
                .compact_header()
                .steady_size()
                .show(ctx, palette, visible, |ui| {
                    tabs(
                        ui,
                        &RowStyle {
                            palette,
                            icons,
                            prefs,
                            facts,
                        },
                        id,
                        name,
                        pane,
                        &mut self.next,
                        &mut frame,
                        &mut self.dragging,
                        &mut self.hovering,
                        preview.as_ref().filter(|p| p.host == id),
                        &self.store.layout,
                        inv,
                        avatar,
                    );
                    if pane.tabs.is_empty() {
                        close = true;
                        return;
                    }
                    if let Some(error) = &self.store.error {
                        ui.colored_label(ui.visuals().warn_fg_color, tr!("organized-save-error", error = error.clone()));
                    }
                    let tab = &mut pane.tabs[pane.active];
                    ui.push_id(("explorer-tab", tab.id), |ui| {
                        organized::show(
                            ui,
                            palette,
                            icons,
                            inv,
                            avatar,
                            facts,
                            prefs,
                            &mut self.store.layout,
                            &mut tab.state,
                            native,
                            hover_images,
                            actions,
                        );
                    });
                    if let Some((place, raw)) = tab.state.pending_open_tab.take() {
                        pane.add_mode(&mut self.next, place, raw);
                    }
                });
            if close {
                *visible = false;
            }
        };
        draw(0, "inventory", home, &mut self.main, open);
        for w in &mut self.detached {
            draw(w.id, &format!("inventory-explorer-{}", w.id), w.pos, &mut w.pane, &mut w.open);
        }
        self.main.finish_inline_creations(inv, native.pending.is_some(), *open, actions);
        for w in &mut self.detached {
            w.pane.finish_inline_creations(inv, native.pending.is_some(), w.open, actions);
        }
        self.detached.retain(|w| w.open);
        if frame.over_item_tab.is_none() {
            self.hovering = None;
        }
        self.update_tab_drag(ctx, &frame, cancel_drag);
        self.strips = frame.strips;
        if let Some(op) = frame.transfer {
            let from_main = op.from == 0;
            let to_main = matches!(op.to, Destination::Pane(0, _));
            self.transfer(op);
            if from_main || to_main {
                *open = !self.main.tabs.is_empty();
            }
            ctx.request_repaint();
        }
        if self.main.tabs.is_empty() {
            *open = false;
        }
        self.store.save();
    }

    fn update_tab_drag(&mut self, ctx: &Context, frame: &Frame, escape: bool) {
        let Some(mut drag) = self.dragging.take() else { return };
        let (pos, down, released, focused) = ctx.input(|i| {
            (
                tab_pointer_pos(i),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.focused,
            )
        });
        let cancel = escape || !focused || pos.is_none() || (!down && !released);
        if cancel {
            if drag.host != drag.origin {
                self.transfer(Transfer {
                    from: drag.host,
                    tab: drag.tab,
                    to: Destination::Pane(drag.origin, drag.original_next),
                });
            } else if drag.moving_window {
                widgets::move_floater(ctx, &window_name(drag.origin), drag.original_pos);
            } else if let Some(pane) = self.pane(drag.origin) {
                pane.reorder(drag.tab, drag.original_next);
            }
            if let Some(pane) = self.pane(drag.origin)
                && let Some(i) = pane.tabs.iter().position(|t| t.id == drag.original_active)
            {
                pane.active = i;
            }
            ctx.request_repaint();
            return;
        }
        let pos = pos.unwrap();
        let own_strip = frame.strips.iter().find(|s| s.0 == drag.host);
        if let Some((_, layer, strip, slots)) = own_strip
            && !drag.moving_window
            && strip.expand2(vec2(8.0, 10.0)).contains(pos)
            && ctx.layer_id_at(pos) == Some(*layer)
        {
            let before = insertion(slots, pos, drag.tab);
            if let Some(pane) = self.pane(drag.host) {
                pane.reorder(drag.tab, before);
            }
        } else {
            let target = dock_target(ctx, &frame.strips, &drag, pos);
            if let Some((target_id, before)) = target
                && released
            {
                self.transfer(Transfer {
                    from: drag.host,
                    tab: drag.tab,
                    to: Destination::Pane(target_id, before),
                });
                ctx.move_to_top(LayerId::new(egui::Order::Middle, egui::Id::unique(window_name(target_id))));
            }
            if !drag.moving_window && !(released && target.is_some()) {
                // A detached tab occupies slot zero, even if the original strip was scrolled.
                let Some((_, _, strip, _)) = own_strip else { return };
                drag.window_grab = strip.min - drag.original_pos + drag.tab_grab;
                self.transfer(Transfer {
                    from: drag.host,
                    tab: drag.tab,
                    to: Destination::Window(pos - drag.window_grab),
                });
                drag.host = self.next;
                drag.moving_window = true;
                let name = window_name(drag.host);
                widgets::move_floater(ctx, &name, pos - drag.window_grab);
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(name)));
            }
        }
        if down && !released {
            self.dragging = Some(drag);
        }
        ctx.request_repaint();
    }
}

fn tab_pointer_pos(input: &egui::InputState) -> Option<Pos2> {
    super::drag::pointer::read(input).map(|pointer| pointer.position)
}

fn dock_target(ctx: &Context, strips: &[Strip], drag: &TabDrag, pos: Pos2) -> Option<(u64, Option<u64>)> {
    // Look through the moving window, but never through a popup or unrelated floater.
    let moving_layer = LayerId::new(egui::Order::Middle, egui::Id::unique(window_name(drag.host)));
    let top = ctx.memory(|m| {
        m.layer_ids()
            .filter(|l| {
                !(drag.moving_window && *l == moving_layer)
                    && l.order != egui::Order::Tooltip
                    && m.areas().is_visible(l)
                    && m.area_rect(l.id).is_some_and(|r| r.contains(pos))
            })
            .last()
    });
    let (id, _, _, slots) = strips
        .iter()
        .find(|(id, layer, rect, _)| *id != drag.host && Some(*layer) == top && rect.contains(pos))?;
    Some((*id, insertion(slots, pos, drag.tab)))
}

fn insertion(slots: &[(Rect, u64)], pos: Pos2, moving: u64) -> Option<u64> {
    slots
        .iter()
        .find(|(r, id)| *id != moving && pos.x < r.center().x)
        .map(|(_, id)| *id)
}

fn close_routed_tab(ctx: &Context, name: &str, pane: &mut Pane) -> bool {
    if crate::ui::close_key::closes_every_window(ctx) || widgets::floater_minimized(ctx, name) {
        return false;
    }
    let close = crate::ui::close_key::take(ctx, egui::Id::unique(name));
    // The shared router reserves Ctrl+W before a popup can read it. Take
    // this window's request without closing the tab behind an open menu.
    if close && !egui::Popup::is_any_open(ctx) {
        pane.close_active();
        return true;
    }
    false
}

// Only a visible Explorer tab registers a window. A stock inventory, a
// minimized floater, a popup and an unrelated frontmost window do not claim
// native accelerators such as Ctrl+T (Conversations elsewhere in the viewer).
pub(crate) fn explorer_owns_keys(ctx: &Context) -> bool {
    keyboard_layer(ctx).is_some()
}

/// Only reserve object-editing shortcuts for the file pane. Text fields keep their normal clipboard and Delete keys.
pub(crate) fn explorer_file_focus(ctx: &Context) -> bool {
    let Some(layer) = keyboard_layer(ctx) else { return false };
    let Some(id) = ctx.data(|d| d.get_temp::<egui::Id>(layer.id.with("explorer-active-list-focus"))) else {
        return false;
    };
    ctx.memory(|m| m.has_focus(id))
}

fn keyboard_layer(ctx: &Context) -> Option<LayerId> {
    if egui::Popup::is_any_open(ctx) || ctx.memory(|m| m.top_modal_layer()).is_some() {
        return None;
    }
    let front = crate::ui::close_key::frontmost_window(ctx)?;
    let focused = ctx.memory(|m| m.focused());
    if let Some(field) = focused
        && ctx.read_response(field).is_some_and(|r| r.layer_id != front)
        && (egui::TextEdit::load_state(ctx, field).is_some() || crate::ui::scripts::editor_focused(ctx))
    {
        return None;
    }
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<u64>(front.id.with("organized-keys")).filter(|at| at + 1 >= frame))?;
    Some(front)
}

fn pressed(ui: &Ui, mods: Modifiers, key: Key) -> bool {
    ui.input_mut(|i| {
        let mut hit = false;
        i.events.retain(|event| {
            let mine = matches!(event, egui::Event::Key {key:k,pressed:true,repeat:false,modifiers,..} if *k==key && modifiers.matches_exact(mods));
            hit |= mine;
            !mine
        });
        hit
    })
}

#[derive(Clone, Copy, PartialEq)]
enum TabRole {
    Background,
    Active,
    Incoming,
}

fn tab_buttons(
    ui: &mut Ui,
    art: &RowStyle,
    tab_id: u64,
    title: &str,
    glyph: Glyph,
    role: TabRole,
    width: f32,
    drag: Option<&TabDrag>,
) -> (Rect, egui::Response, egui::Response) {
    let p = art.palette;
    let docking = role == TabRole::Incoming;
    let selected = role != TabRole::Background;
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 13.0).max(30.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let close_area = Rect::from_min_max(pos2(rect.right() - 27.0, rect.top()), rect.max);
    let id = ui.make_persistent_id(("explorer-tab", tab_id));
    let body = ui.interact(
        Rect::from_min_max(rect.min, pos2(close_area.left(), rect.bottom())),
        id,
        if docking {
            egui::Sense::hover()
        } else {
            egui::Sense::click_and_drag()
        },
    );
    let close = ui.interact(
        close_area.shrink2(vec2(3.0, 4.0)),
        id.with("close"),
        if docking { egui::Sense::hover() } else { egui::Sense::click() },
    );
    body.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, title));
    close.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tr!("organized-close-tab")));
    let preview = drag
        .filter(|d| d.tab == tab_id && !d.moving_window)
        .and_then(|d| ui.input(tab_pointer_pos).map(|pos| pos - d.tab_grab));
    if ui.is_rect_visible(rect) || preview.is_some() {
        let hovered = body.hovered() || close.hovered();
        // Tint only the tab surface. The gaps, corners and space after + keep
        // the same glass as the floater's title and navigation rows.
        let fill = if (selected && p.high_contrast) || preview.is_some() {
            p.raised
        } else if selected {
            p.violet_light.gamma_multiply(0.16)
        } else if hovered || body.has_focus() {
            p.ink.gamma_multiply(0.09)
        } else {
            p.ink.gamma_multiply(0.035)
        };
        let text = crate::interface_colors::readable(if selected || hovered { p.ink } else { p.muted }, p.panel, 4.5);
        let radius = egui::CornerRadius {
            nw: ui.visuals().widgets.inactive.corner_radius.nw,
            ne: ui.visuals().widgets.inactive.corner_radius.ne,
            sw: 0,
            se: 0,
        };
        let face = if let Some(min) = preview {
            // A gap in the scrolling strip makes the insertion slot unambiguous.
            ui.painter().rect_filled(rect, radius, p.violet_light.gamma_multiply(0.08));
            Rect::from_min_size(min, rect.size())
        } else {
            rect
        };
        let painter = if preview.is_some() {
            ui.ctx()
                .layer_painter(LayerId::new(egui::Order::Tooltip, egui::Id::unique("explorer-drag-tab")))
        } else {
            ui.painter().clone()
        };
        painter.rect_filled(face, radius, fill);
        if docking || body.has_focus() || (p.high_contrast && selected) {
            let outline = if docking { p.violet_light } else { text };
            painter.rect_stroke(face, radius, egui::Stroke::new(1.5, outline), egui::StrokeKind::Inside);
        }
        let icon_side = (face.height() - 12.0).clamp(14.0, 20.0);
        let icon = Rect::from_center_size(
            pos2(face.left() + 9.0 + icon_side * 0.5, face.center().y),
            vec2(icon_side, icon_side),
        );
        art.glyph(&painter, glyph, icon);
        let left = icon.right() + 6.0;
        let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), egui::TextStyle::Button.resolve(ui.style()), text);
        job.wrap.max_width = (face.right() - close_area.width() - 4.0 - left).max(1.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = painter.layout_job(job);
        widgets::paint_row_text(&painter, galley, left, face.center().y, text);
        let center = close.rect.center() + (face.min - rect.min);
        let close_ink = if close.hovered() || close.has_focus() {
            let fill = ui.visuals().widgets.hovered.bg_fill;
            painter.rect_filled(
                Rect::from_center_size(center, vec2(18.0, 18.0)),
                ui.visuals().widgets.inactive.corner_radius,
                fill,
            );
            crate::interface_colors::readable(p.ink, fill, 4.5)
        } else {
            text
        };
        organized::paint_tabler(&painter, "x", Rect::from_center_size(center, vec2(12.0, 12.0)), close_ink);
    }
    (rect, body.on_hover_text(title), close.on_hover_text(tr!("organized-close-tab")))
}

fn tabs(
    ui: &mut Ui,
    art: &RowStyle,
    host: u64,
    name: &str,
    pane: &mut Pane,
    next: &mut u64,
    frame: &mut Frame,
    dragging: &mut Option<TabDrag>,
    hovering: &mut Option<(u64, f64)>,
    preview: Option<&DockPreview>,
    layout: &Preferences,
    inv: &Inventory,
    avatar: &str,
) {
    let palette = art.palette;
    let mut add = false;
    let mut close = None;
    let frame_nr = ui.ctx().cumulative_frame_nr();
    let marker = ui.layer_id().id.with("organized-keys");
    ui.data_mut(|data| data.insert_temp(marker, frame_nr));
    super::explorer_key_debug::phase(ui.ctx(), "tabs", Some(ui.layer_id()), None);
    if dragging.is_none()
        && ui.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
        })
        && keyboard_layer(ui.ctx()).is_some_and(|layer| layer.id == egui::Id::unique(name))
    {
        add = pressed(ui, Modifiers::COMMAND, Key::T);
        if pressed(ui, Modifiers::COMMAND, Key::W) {
            close = Some(pane.tabs[pane.active].id);
        }
        let n = pane.tabs.len();
        if pressed(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab) {
            pane.active = (pane.active + n - 1) % n;
        } else if pressed(ui, Modifiers::COMMAND, Key::Tab) {
            pane.active = (pane.active + 1) % n;
        }
        let state = &mut pane.tabs[pane.active].state;
        if pressed(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::N) {
            organized::folder::begin(inv, layout, state);
        }
        if pressed(ui, Modifiers::COMMAND, Key::L) {
            state.focus_address();
        }
        if pressed(ui, Modifiers::COMMAND, Key::F) || pressed(ui, Modifiers::NONE, Key::F3) {
            state.focus_search();
        }
        for key in [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp] {
            if pressed(ui, Modifiers::ALT, key) {
                state.navigate_key(key, inv, layout);
            }
        }
    }
    let mut slots = Vec::new();
    let selected = pane.tabs[pane.active].id;
    let reveal = pane.revealed != Some(selected);
    if reveal && pane.revealed.is_some() {
        pane.tabs[pane.active].state.focus_contents = true;
    }
    pane.revealed = Some(selected);
    let strip = ui
        .horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            let width = (ui.available_width() - 30.0 - spacing).max(40.0);
            let gap = 3.0;
            let count = pane.tabs.len() + usize::from(preview.is_some());
            let tab_width = ((width - gap * (count - 1) as f32) / count as f32).clamp(104.0, 260.0);
            let strip_width = width.min(tab_width * count as f32 + gap * (count - 1) as f32);
            let tab_height = (ui.text_style_height(&egui::TextStyle::Button) + 13.0).max(30.0);
            let tabs_top = ui.next_widget_position().y;
            egui::ScrollArea::horizontal()
                .id_salt(("explorer-tab-strip", host))
                .max_width(strip_width)
                .min_scrolled_width(strip_width)
                .max_height((ui.text_style_height(&egui::TextStyle::Button) + 23.0).max(40.0))
                .animated(false)
                .auto_shrink([false, true])
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: egui::scroll_area::DragScroll::Never,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    if dragging.is_some()
                        && tab_width * count as f32 > strip_width
                        && let Some(pointer) = ui.input(tab_pointer_pos)
                    {
                        let viewport = ui.clip_rect();
                        if viewport.contains(pointer) {
                            let direction = if pointer.x < viewport.left() + 18.0 {
                                1.0
                            } else if pointer.x > viewport.right() - 18.0 {
                                -1.0
                            } else {
                                0.0
                            };
                            ui.scroll_with_delta(vec2(direction * 420.0 * ui.input(|i| i.stable_dt.min(0.05)), 0.0));
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for i in 0..=pane.tabs.len() {
                            let tab = pane.tabs.get(i);
                            if let Some(incoming) = preview.filter(|p| p.before == tab.map(|t| t.id)) {
                                let glyph = art.location(inv, layout, incoming.location, true);
                                let (slot, _, _) =
                                    tab_buttons(ui, art, incoming.tab, &incoming.title, glyph, TabRole::Incoming, tab_width, None);
                                let viewport = ui.clip_rect();
                                if slot.left() < viewport.left() - 1.0 || slot.right() > viewport.right() + 1.0 {
                                    ui.scroll_to_rect_animation(slot, None, egui::style::ScrollAnimation::none());
                                }
                            }
                            let Some(tab) = tab else { break };
                            let title = tab.state.title(inv, avatar, layout);
                            let active = i == pane.active && preview.is_none();
                            let glyph = art.location(inv, layout, tab.state.location(), active);
                            let role = if active { TabRole::Active } else { TabRole::Background };
                            let (rect, response, x) = tab_buttons(ui, art, tab.id, &title, glyph, role, tab_width, dragging.as_ref());
                            if x.clicked() {
                                close = Some(tab.id);
                            }
                            if reveal && selected == tab.id {
                                ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
                            }
                            slots.push((rect, tab.id));
                            if dragging.is_none() && response.clicked() {
                                pane.active = i;
                            }
                            if response.middle_clicked() {
                                close = Some(tab.id);
                            }
                            if response.drag_started_by(egui::PointerButton::Primary) && dragging.is_none() {
                                let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(rect.center());
                                let original_pos = ui.memory(|m| m.area_rect(ui.layer_id().id)).map_or(rect.min, |r| r.min);
                                *dragging = Some(TabDrag {
                                    tab: tab.id,
                                    host,
                                    origin: host,
                                    original_next: pane.tabs.get(i + 1).map(|t| t.id),
                                    original_active: selected,
                                    original_pos,
                                    tab_grab: press - rect.min,
                                    window_grab: press - original_pos,
                                    moving_window: pane.tabs.len() == 1,
                                });
                            }
                            if response.contains_pointer() && organized::dragging(ui.ctx()) {
                                frame.over_item_tab = Some(tab.id);
                                let now = ui.input(|i| i.time);
                                if hovering.is_some_and(|(id, since)| id == tab.id && now - since >= 0.42) {
                                    pane.active = i;
                                } else if hovering.is_none_or(|(id, _)| id != tab.id) {
                                    *hovering = Some((tab.id, now));
                                }
                                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                            }
                            organized::context_menu(&response, palette, |ui| {
                                if ui.button(tr!("organized-new-tab")).clicked() {
                                    add = true;
                                    ui.close();
                                }
                                if ui.button(tr!("organized-detach")).clicked() {
                                    frame.transfer = Some(Transfer {
                                        from: host,
                                        tab: tab.id,
                                        to: Destination::Window(response.rect.left_bottom()),
                                    });
                                    ui.close();
                                }
                                if host != 0 && ui.button(tr!("organized-dock")).clicked() {
                                    frame.transfer = Some(Transfer {
                                        from: host,
                                        tab: tab.id,
                                        to: Destination::Pane(0, None),
                                    });
                                    ui.close();
                                }
                                if ui.button(tr!("organized-close-tab")).clicked() {
                                    close = Some(tab.id);
                                    ui.close();
                                }
                            });
                        }
                    });
                });
            let (plus_space, _) = ui.allocate_exact_size(vec2(29.0, tab_height), egui::Sense::hover());
            let center = pos2(
                plus_space.center().x,
                slots.first().map_or(tabs_top + tab_height * 0.5, |(r, _)| r.center().y),
            );
            let rect = Rect::from_center_size(center, vec2(28.0, 28.0));
            let plus = ui.interact(rect, ui.make_persistent_id("new-tab"), egui::Sense::click());
            plus.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tr!("organized-new-tab")));
            if plus.hovered() || plus.has_focus() {
                ui.painter()
                    .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, palette.ink.gamma_multiply(0.09));
            }
            organized::paint_tabler(ui.painter(), "plus", Rect::from_center_size(center, vec2(15.0, 15.0)), palette.ink);
            if plus.on_hover_text(tr!("organized-new-tab")).clicked() {
                add = true;
            }
        })
        .response
        .rect;
    frame.strips.push((
        host,
        ui.layer_id(),
        Rect::from_min_max(strip.min, pos2(ui.max_rect().right(), strip.bottom())),
        slots,
    ));
    if let Some(id) = close {
        pane.close(id);
    }
    if add {
        pane.add(next, Location::Home);
    }
}

impl Host {
    /// Deliver the reply to its originating tab, including a detached or inactive tab.
    fn folder_result(&mut self, inv: &Inventory, request: &super::ExplorerFolderRequest, result: Result<Uuid, &str>, demo: bool) {
        if request.root != inv.root || self.root != inv.root {
            return;
        }
        let destination = organized::folder::confirmed_destination(inv, &mut self.store.layout, request, result);
        self.store.save();
        for tab in self
            .main
            .tabs
            .iter_mut()
            .chain(self.detached.iter_mut().flat_map(|w| w.pane.tabs.iter_mut()))
        {
            if organized::folder::finish(&mut tab.state, request, destination, &result, demo) {
                break;
            }
        }
    }
}

impl InventoryUi {
    pub(crate) fn explorer_home_backing(&self, inv: &Inventory) -> Option<Uuid> {
        super::organized_model::home_folder(inv, &self.organized_host.store.layout)
    }

    pub(crate) fn bind_explorer_home(&mut self, inv: &Inventory, root: Uuid, id: Uuid) -> bool {
        if self.organized_host.root != root
            || inv.root != root
            || !inv
                .folders
                .get(&id)
                .is_some_and(|f| f.info.parent == root && f.info.type_default == -1 && !f.library)
        {
            return false;
        }
        self.organized_host.store.layout.set_home_folder(id);
        self.organized_host.store.save();
        true
    }

    pub(crate) fn explorer_folder_result(
        &mut self,
        inv: &Inventory,
        request: &super::ExplorerFolderRequest,
        result: Result<Uuid, &str>,
        demo: bool,
    ) {
        self.organized_host.folder_result(inv, request, result, demo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn organized_browser_tabs_and_rows_use_distinct_readable_skin_surfaces() {
        use crate::interface_colors::contrast;
        use egui::{Color32, Shape};
        let dark = crate::theme::Theme::default().skin_palette();
        let light = Palette {
            panel: Color32::from_gray(242),
            raised: Color32::from_gray(225),
            ink: Color32::BLACK,
            muted: Color32::from_gray(88),
            muted_dim: Color32::from_gray(150),
            ..dark
        };
        for palette in [dark, light, crate::high_contrast::palette()] {
            let ctx = Context::default();
            let mut tabs = [Rect::NOTHING; 2];
            let mut row = Rect::NOTHING;
            let mut ink = Color32::TRANSPARENT;
            let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                for (i, r) in tabs.iter_mut().enumerate() {
                    let art = RowStyle {
                        palette: &palette,
                        icons: &Icons::default(),
                        prefs: &Default::default(),
                        facts: &Default::default(),
                    };
                    let glyph = art.location(&Inventory::default(), &Preferences::default(), Location::Home, i == 0);
                    let role = if i == 0 { TabRole::Active } else { TabRole::Background };
                    let (tab, body, close) = tab_buttons(ui, &art, i as u64, "Inventory folders", glyph, role, 104.0, None);
                    assert!(body.rect.right() < close.rect.left() && tab.contains_rect(close.rect));
                    *r = tab;
                }
                let (r, hit) = ui.allocate_exact_size(vec2(104.0, 26.0), egui::Sense::click());
                row = r;
                ink = organized::paint_row_selection(ui, &palette, &hit, r, true);
            });
            frame.textures_delta.clear();
            let mut fills = Vec::new();
            for tab in tabs {
                let face = frame
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        Shape::Rect(face) if face.rect == tab && face.fill != Color32::TRANSPARENT => Some(face),
                        _ => None,
                    })
                    .expect("the tab face occupies its full hit target");
                assert_eq!(face.corner_radius.nw, face.corner_radius.ne);
                assert!(face.corner_radius.nw > 0);
                assert_eq!((face.corner_radius.sw, face.corner_radius.se), (0, 0));
                fills.push(face.fill);
            }
            assert_ne!(fills[0], fills[1]);
            let selected = frame
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    Shape::Rect(face) if face.rect == row && face.stroke.width >= 1.0 => Some(face),
                    _ => None,
                })
                .expect("selected row has a visible outline");
            assert_ne!(selected.fill, palette.violet);
            assert!(contrast(ink, selected.fill) >= 4.5);
            assert!(contrast(selected.stroke.color, palette.panel) >= 3.0);
            if !palette.high_contrast {
                assert_ne!(fills[0], palette.violet);
            }
        }
    }

    #[test]
    fn organized_tab_icons_follow_the_location_and_label_stays_left_at_different_widths() {
        use super::super::organized_model::Group;
        let ctx = Context::default();
        let icons = Icons::load(&ctx, None);
        let art = RowStyle {
            palette: &crate::theme::Theme::default().skin_palette(),
            icons: &icons,
            prefs: &Default::default(),
            facts: &Default::default(),
        };
        let mut layout = Preferences::default();
        let folder = Location::Folder(Uuid::new_v4());
        layout.set_folder_icon(folder, Some("star-filled"));
        let inv = Inventory::default();
        for (place, expected) in [
            (Location::Home, "home-filled"),
            (Location::Group(Group::Appearance), "shirt-filled"),
            (folder, "star-filled"),
        ] {
            for width in [104.0, 260.0] {
                let mut rect = Rect::NOTHING;
                let title = "Region builds and staged inventory";
                let mut render = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let glyph = art.location(&inv, &layout, place, true);
                    rect = tab_buttons(ui, &art, 7, title, glyph, TabRole::Active, width, None).0;
                });
                render.textures_delta.clear();
                let icon = render
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Mesh(mesh) if mesh.texture_id == icons.get(expected).unwrap().id() => {
                            Some(Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()))
                        }
                        _ => None,
                    })
                    .expect("location symbol was painted");
                let text = render
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == title => Some(t),
                        _ => None,
                    })
                    .expect("caption was painted");
                assert!((icon.left() - rect.left() - 9.0).abs() < 1.0);
                assert!((icon.center().y - rect.center().y).abs() < 1.0);
                assert!((text.pos.x - icon.right() - 6.0).abs() < 1.0);
                assert!(text.pos.x + text.galley.size().x <= rect.right() - 27.0);
            }
        }
    }

    #[test]
    fn organized_native_tab_keys_inside_x_and_overflow_keep_the_right_tab_active() {
        use egui::{Event, Id, PointerButton, RawInput};
        let ctx = Context::default();
        let inv = Inventory::default();
        let layout = Preferences::default();
        let name = "organized-shortcut-test";
        let mut pane = Pane::default();
        let mut next = 0;
        pane.add(&mut next, Location::Home);
        let mut clock = 0;
        let mut tick = |pane: &mut Pane, events: Vec<Event>, modal: bool| {
            clock += 1;
            let mut frame = Frame::default();
            let mut out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 380.0))),
                    time: Some(clock as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::Area::new(Id::unique("foreground-toolbar"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(pos2(520.0, 75.0))
                        .show(ui.ctx(), |ui| {
                            let _ = ui.button("HUD");
                        });
                    egui::Area::new(Id::unique("passive-tab-tooltip"))
                        .order(egui::Order::Tooltip)
                        .interactable(false)
                        .fixed_pos(pos2(520.0, 125.0))
                        .show(ui.ctx(), |ui| {
                            ui.label("Tooltip");
                        });
                    if modal {
                        egui::Modal::new(Id::unique("test-modal")).show(ui.ctx(), |ui| {
                            ui.label("Modal");
                        });
                    }
                    crate::ui::close_key::begin(ui.ctx());
                    if close_routed_tab(ui.ctx(), name, pane) && pane.tabs.is_empty() {
                        return;
                    }
                    egui::Window::new(name)
                        .id(Id::unique(name))
                        .fixed_pos(pos2(10.0, 10.0))
                        .fixed_size(vec2(500.0, 180.0))
                        .show(ui.ctx(), |ui| {
                            ui.set_width(480.0);
                            crate::ui::close_key::shown(ui.ctx(), Id::unique(name));
                            tabs(
                                ui,
                                &RowStyle {
                                    palette: &crate::theme::Theme::default().skin_palette(),
                                    icons: &Icons::default(),
                                    prefs: &Default::default(),
                                    facts: &Default::default(),
                                },
                                0,
                                name,
                                pane,
                                &mut next,
                                &mut frame,
                                &mut None,
                                &mut None,
                                None,
                                &layout,
                                &inv,
                                "Tester",
                            );
                        });
                },
            );
            out.textures_delta.clear();
            frame.strips.pop().map(|(_, _, _, slots)| slots).unwrap_or_default()
        };
        let key = |key, modifiers| Event::Key {
            key,
            modifiers,
            physical_key: None,
            pressed: true,
            repeat: false,
        };
        let release = |key, modifiers| Event::Key {
            key,
            modifiers,
            physical_key: None,
            pressed: false,
            repeat: false,
        };
        let click = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        for _ in 0..4 {
            tick(&mut pane, vec![], false);
        }
        assert!(explorer_owns_keys(&ctx));
        let first = pane.tabs[0].id;
        tick(&mut pane, vec![key(Key::T, Modifiers::COMMAND)], false);
        assert_eq!(pane.tabs.len(), 2);
        let second = pane.tabs[pane.active].id;
        let slots = tick(&mut pane, vec![release(Key::T, Modifiers::COMMAND)], false);
        assert_eq!(slots.len(), 2);
        assert!(slots[0].0.width() > 180.0);
        let background_x = pos2(slots[0].0.right() - 13.0, slots[0].0.center().y);
        tick(&mut pane, vec![Event::PointerMoved(background_x)], false);
        tick(&mut pane, vec![click(background_x, true)], false);
        tick(&mut pane, vec![click(background_x, false)], false);
        assert_eq!(pane.tabs.len(), 1);
        assert_eq!(pane.tabs[pane.active].id, second);
        assert_ne!(first, second);
        for _ in 0..7 {
            tick(&mut pane, vec![key(Key::T, Modifiers::COMMAND)], false);
            tick(&mut pane, vec![release(Key::T, Modifiers::COMMAND)], false);
        }
        assert_eq!(pane.tabs.len(), 8);
        let slots = tick(&mut pane, vec![], false);
        assert!(slots[0].0.right() < slots[7].0.left());
        let window = egui::AreaState::load(&ctx, Id::unique(name)).unwrap().rect();
        assert!(
            slots[7].0.right() < window.right() - 27.0,
            "active tab must scroll into view: {:?} in {:?}",
            slots[7].0,
            window
        );
        let last = pane.tabs[pane.active].id;
        tick(&mut pane, vec![key(Key::Tab, Modifiers::COMMAND)], false);
        assert_eq!(pane.tabs[pane.active].id, second);
        tick(&mut pane, vec![release(Key::Tab, Modifiers::COMMAND)], false);
        tick(&mut pane, vec![key(Key::Tab, Modifiers::COMMAND | Modifiers::SHIFT)], false);
        assert_eq!(pane.tabs[pane.active].id, last);
        tick(&mut pane, vec![], true);
        assert!(!explorer_owns_keys(&ctx));
        tick(
            &mut pane,
            vec![key(Key::T, Modifiers::COMMAND), key(Key::W, Modifiers::COMMAND)],
            true,
        );
        assert_eq!(pane.tabs.len(), 8);
        tick(
            &mut pane,
            vec![release(Key::W, Modifiers::COMMAND), release(Key::T, Modifiers::COMMAND)],
            false,
        );
        for _ in 0..3 {
            tick(&mut pane, vec![], false);
        }
        let popup = Id::unique("explorer-menu");
        egui::Popup::open_id(&ctx, popup);
        assert!(!explorer_owns_keys(&ctx));
        tick(
            &mut pane,
            vec![key(Key::W, Modifiers::COMMAND), key(Key::T, Modifiers::COMMAND)],
            false,
        );
        assert_eq!(pane.tabs.len(), 8, "the open menu retains keyboard ownership");
        egui::Popup::close_id(&ctx, popup);
        tick(
            &mut pane,
            vec![release(Key::W, Modifiers::COMMAND), release(Key::T, Modifiers::COMMAND)],
            false,
        );
        while !pane.tabs.is_empty() {
            let count = pane.tabs.len();
            tick(&mut pane, vec![key(Key::W, Modifiers::COMMAND)], false);
            assert_eq!(pane.tabs.len(), count - 1);
            if !pane.tabs.is_empty() {
                tick(&mut pane, vec![release(Key::W, Modifiers::COMMAND)], false);
            }
        }
    }

    struct DragFixture {
        ctx: Context,
        host: Host,
        open: bool,
        frame: u32,
        render: egui::FullOutput,
    }

    impl DragFixture {
        fn new(locations: &[Location]) -> Self {
            let mut f = Self {
                ctx: Context::default(),
                host: Host::default(),
                open: true,
                frame: 0,
                render: Default::default(),
            };
            for &location in locations {
                f.host.main.add(&mut f.host.next, location);
            }
            for _ in 0..5 {
                f.tick(vec![]);
            }
            f
        }

        fn tick(&mut self, events: Vec<egui::Event>) {
            self.frame += 1;
            self.render = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 1000.0))),
                    time: Some(self.frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    crate::ui::close_key::begin(ui.ctx());
                    self.host.show(
                        ui.ctx(),
                        &crate::theme::Theme::default().skin_palette(),
                        &Icons::default(),
                        &mut Inventory::default(),
                        "Drag fixture home",
                        &mut Default::default(),
                        &Default::default(),
                        &mut Default::default(),
                        &mut self.open,
                        &Default::default(),
                        &mut Vec::new(),
                    );
                },
            );
            self.render.textures_delta.clear();
        }

        fn title(&self, idx: usize) -> Pos2 {
            let title = self.host.main.tabs[idx]
                .state
                .title(&Inventory::default(), "Drag fixture home", &self.host.store.layout);
            self.render
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(text) if text.galley.text() == title => Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                })
                .expect("visible tab title")
        }

        fn button(&mut self, pos: Pos2, pressed: bool) {
            self.tick(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }

        fn escape(&mut self) {
            self.tick(vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
        }

        fn window(&self, id: u64) -> Rect {
            self.ctx.memory(|m| m.area_rect(egui::Id::unique(window_name(id))).unwrap())
        }

        fn ids(&self) -> Vec<u64> {
            self.host.main.tabs.iter().map(|t| t.id).collect()
        }
    }

    #[test]
    fn organized_tabs_reorder_while_held_and_escape_restores_a_torn_off_tab() {
        let mut f = DragFixture::new(&[Location::Home, Location::Library, Location::MyFolders]);
        let ids = f.ids();
        let from = f.title(0);
        let second = f.host.strips[0].3[1].0;
        let after_second = second.center() + vec2(16.0, 0.0);
        f.button(from, true);
        f.tick(vec![egui::Event::PointerMoved(after_second)]);
        f.tick(vec![]);
        assert_eq!(f.ids(), [ids[1], ids[0], ids[2]]);
        assert_eq!(f.host.main.tabs[f.host.main.active].id, ids[2]);
        assert!(f.host.dragging.as_ref().is_some_and(|d| d.tab == ids[0]));
        let outside = after_second + vec2(-440.0, 120.0);
        f.tick(vec![egui::Event::PointerMoved(outside)]);
        f.tick(vec![]);
        assert_eq!(f.host.detached.len(), 1);
        assert_eq!(f.host.detached[0].pane.tabs[0].id, ids[0]);
        let moving = f.host.detached[0].id;
        let start = f.window(moving).min;
        f.tick(vec![egui::Event::PointerMoved(outside + vec2(-50.0, 65.0))]);
        f.tick(vec![]);
        assert!(f.window(moving).min.distance(start + vec2(-50.0, 65.0)) < 3.0);
        f.escape();
        f.button(outside, false);
        assert_eq!(f.ids(), ids);
        assert_eq!(f.host.main.tabs[f.host.main.active].id, ids[2]);
        assert!(f.host.detached.is_empty());
    }

    #[test]
    fn switching_inventory_style_cancels_a_tab_drag_and_keeps_its_original_session() {
        let mut f = DragFixture::new(&[Location::Home, Location::Library]);
        let ids = f.ids();
        let from = f.title(0);
        f.button(from, true);
        let outside = f.host.strips[0].2.left_bottom() + vec2(-90.0, 75.0);
        f.tick(vec![egui::Event::PointerMoved(outside)]);
        f.tick(vec![]);
        assert!(f.host.dragging.is_some() && f.host.has_detached());
        f.host.suspend(&f.ctx, &Inventory::default(), false, &mut Vec::new());
        assert!(f.host.dragging.is_none() && f.ctx.dragged_id().is_none());
        assert_eq!(f.ids(), ids);
        assert!(!f.host.has_detached() && f.host.strips.is_empty());
    }

    #[test]
    fn organized_single_tab_moves_its_window_live_and_docks_through_its_own_layer() {
        let mut f = DragFixture::new(&[Location::Home]);
        let original = f.window(0).min;
        let tab = f.host.main.tabs[0].id;
        let at = f.title(0);
        f.button(at, true);
        let moved = at + vec2(-90.0, 75.0);
        f.tick(vec![egui::Event::PointerMoved(moved)]);
        f.tick(vec![]);
        assert!(f.window(0).min.distance(original + vec2(-90.0, 75.0)) < 3.0);
        assert!(f.host.detached.is_empty());
        f.escape();
        f.button(moved, false);
        assert!(f.window(0).min.distance(original) < 3.0);
        f.host.open_window(&f.ctx, Uuid::nil());
        for _ in 0..4 {
            f.tick(vec![]);
        }
        let other = f.host.detached[0].id;
        let dest = f.window(other).min + (at - original);
        f.button(at, true);
        f.tick(vec![egui::Event::PointerMoved(at + vec2(-20.0, 0.0))]);
        f.tick(vec![egui::Event::PointerMoved(dest)]);
        f.tick(vec![]);
        // Move after button-up, but before the next frame: commit where
        // the button was released and keep the destination preview visible.
        f.tick(vec![
            egui::Event::PointerMoved(dest),
            egui::Event::PointerButton {
                pos: dest,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
            egui::Event::PointerMoved(dest + vec2(310.0, 150.0)),
        ]);
        assert!(
            f.host.strips.iter().all(|s| s.0 != 0),
            "the moving source stayed hidden on its docking frame"
        );
        assert!(!f.open);
        assert_eq!(f.host.detached.len(), 1);
        let pane = &f.host.detached[0].pane;
        assert_eq!(pane.tabs.len(), 2);
        assert_eq!(pane.tabs[pane.active].id, tab);
    }

    #[test]
    fn organized_dock_preview_occupies_destination_slot_until_drop_or_cancel() {
        let mut f = DragFixture::new(&[Location::Library]);
        let label = f.host.main.tabs[0].state.title(&Inventory::default(), "", &f.host.store.layout);
        let dragged = f.host.main.tabs[0].id;
        f.host.open_window(&f.ctx, Uuid::nil());
        let other = f.host.detached[0].id;
        f.host.detached[0].pane.add(&mut f.host.next, Location::MyFolders);
        for _ in 0..5 {
            f.tick(vec![]);
        }
        let strip = |f: &DragFixture| f.host.strips.iter().find(|s| s.0 == other).unwrap().clone();
        let painted_at = |f: &DragFixture| {
            let band = strip(f).2;
            f.render.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label && band.contains(t.pos + t.galley.size() * 0.5) => Some(t.pos),
                _ => None,
            })
        };
        let initial = strip(&f);
        let first = initial.3[0].0;
        let second = initial.3[1].0;
        let at = f.title(0);
        f.button(at, true);
        f.tick(vec![egui::Event::PointerMoved(at + vec2(-20.0, 0.0))]);
        let before = first.left_center() + vec2(20.0, 0.0);
        f.tick(vec![egui::Event::PointerMoved(before)]);
        f.tick(vec![]);
        assert!(
            painted_at(&f).is_some_and(|text| text.x < strip(&f).3[0].0.left()),
            "preview is in the first slot"
        );
        assert!(
            strip(&f).3[0].0.left() > first.left() + 70.0,
            "neighbors move to reserve the incoming slot"
        );
        assert_eq!(f.host.detached[0].pane.tabs.len(), 2);
        assert_eq!(f.host.main.tabs[0].id, dragged);
        let away = before + vec2(0.0, 135.0);
        f.tick(vec![egui::Event::PointerMoved(away)]);
        f.tick(vec![]);
        assert!(painted_at(&f).is_none());
        assert!((strip(&f).3[0].0.left() - first.left()).abs() <= 1.0);
        let after = second.right_center() + vec2(5.0, 0.0);
        f.tick(vec![egui::Event::PointerMoved(after)]);
        f.tick(vec![]);
        assert!(
            painted_at(&f).is_some_and(|text| text.x > strip(&f).3[1].0.right()),
            "preview follows the new insertion boundary"
        );
        f.escape();
        f.button(after, false);
        assert!(painted_at(&f).is_none());
        assert_eq!(f.host.detached[0].pane.tabs.len(), 2);
        assert_eq!(f.host.main.tabs[0].id, dragged);
    }

    #[test]
    fn organized_dock_preview_remains_visible_in_an_overflowing_strip() {
        let mut f = DragFixture::new(&[Location::Library]);
        let label = f.host.main.tabs[0].state.title(&Inventory::default(), "", &f.host.store.layout);
        f.host.open_window(&f.ctx, Uuid::nil());
        let other = f.host.detached[0].id;
        let target = &mut f.host.detached[0].pane;
        for _ in 0..12 {
            target.add(&mut f.host.next, Location::Home);
        }
        target.active = 0;
        for _ in 0..5 {
            f.tick(vec![]);
        }
        let strip = f.host.strips.iter().find(|s| s.0 == other).unwrap().2;
        let at = f.title(0);
        f.button(at, true);
        f.tick(vec![egui::Event::PointerMoved(at + vec2(-20.0, 0.0))]);
        let edge = strip.right_center() - vec2(6.0, 0.0);
        f.tick(vec![egui::Event::PointerMoved(edge)]);
        for _ in 0..20 {
            f.tick(vec![]);
        }
        assert!(
            f.render.shapes.iter().any(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    let center = t.pos + t.galley.size() * 0.5;
                    strip.contains(center) && s.clip_rect.contains(center)
                }
                _ => false,
            }),
            "the preview's caption is visible at the edge of an overflowing tab bar"
        );
        assert_eq!(f.host.detached[0].pane.tabs.len(), 13);
        f.button(edge, false);
        assert_eq!(f.host.detached[0].pane.tabs.len(), 14);
    }

    #[test]
    fn organized_detach_and_dock_keep_tab_and_selection() {
        let mut host = Host::default();
        host.main.add(&mut host.next, Location::Home);
        host.main.add(&mut host.next, Location::Library);
        let tab = host.main.tabs[1].id;
        host.transfer(Transfer {
            from: 0,
            tab,
            to: Destination::Window(Pos2::ZERO),
        });
        assert_eq!(host.main.active, 0);
        assert_eq!(host.detached[0].pane.tabs[0].state.location(), Location::Library);
        let id = host.detached[0].id;
        let before = host.main.tabs[0].id;
        host.transfer(Transfer {
            from: id,
            tab,
            to: Destination::Pane(0, Some(before)),
        });
        assert!(!host.has_detached());
        assert_eq!(host.main.tabs[host.main.active].id, tab);
        assert_eq!(host.main.tabs[1].id, before);
    }
}
