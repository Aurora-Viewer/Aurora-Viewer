//! A transcript owns its menu independently of the virtualized row that
//! opened it. Keep only that target, as LLChatHistoryHeader's avatar menu
//! does (indra/newview/llchathistory.cpp), while messages keep updating.

use crate::theme::Palette;
use crate::world::ChatLine;
use egui::{Id, Pos2, Response, Ui};
use std::cell::Cell;
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct Target {
    pub line: ChatLine,
    /// Snapshot of the transcript selection when its message menu opened.
    pub selected_text: Option<String>,
    /// None: the message or an inline link; Some: its sender's menu.
    pub sender: Option<Uuid>,
}

#[derive(Clone)]
struct Open {
    target: Arc<Target>,
    pos: Pos2,
    seq: Option<u64>,
    popup: Id,
}

fn open(response: &Response, id: Id, line: &ChatLine, sender: Option<Uuid>, seq: Option<u64>, selected_text: Option<String>) {
    if let Some(pos) = response.interact_pointer_pos() {
        response.ctx.data_mut(|d| {
            d.insert_temp(
                id,
                Open {
                    target: Arc::new(Target {
                        line: line.clone(),
                        selected_text,
                        sender,
                    }),
                    pos,
                    seq,
                    popup: id,
                },
            );
        });
        egui::Popup::open_id(&response.ctx, id);
    }
}

pub(super) fn sender(response: &Response, id: Id, avatar: Uuid, line: &ChatLine) {
    if response.secondary_clicked() {
        open(response, id, line, Some(avatar), super::select::current_seq(), None);
    }
}

pub(super) fn message<'l>(
    response: &Response,
    id: Id,
    drawn: &[super::select::Row],
    line_of: impl Fn(u64) -> Option<&'l ChatLine>,
    selected_text: impl Fn(u64) -> Option<String>,
) {
    if response.secondary_clicked()
        && let Some((seq, line)) = response
            .interact_pointer_pos()
            .and_then(|pos| drawn.iter().find(|row| row.rect.contains(pos)))
            .and_then(|row| line_of(row.seq).map(|line| (row.seq, line)))
    {
        open(response, id, line, None, Some(seq), selected_text(seq));
    }
}

pub(super) fn is_open(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<Open>(id))
        .is_some_and(|m| egui::Popup::is_id_open(ctx, m.popup))
}

pub(super) fn selected(ctx: &egui::Context, id: Id) -> Option<u64> {
    ctx.data(|d| d.get_temp::<Open>(id))
        .filter(|m| egui::Popup::is_id_open(ctx, m.popup))
        .and_then(|m| m.seq)
}

thread_local! {
    static INLINE: Cell<Option<Id>> = const { Cell::new(None) };
}

/// Link/preview menus keep their existing actions; their containing message
/// still owns the reading pause and highlight.
pub(super) fn inline(response: &Response) {
    let popup = egui::Popup::default_response_id(response);
    if egui::Popup::is_id_open(&response.ctx, popup) {
        INLINE.set(Some(popup));
    }
}

/// Keep the menu's original row marked even when the pointer is over the
/// popup. Sequence numbers prevent a later, identical message being marked.
pub(super) fn highlight(ui: &mut Ui, id: Id, seq: u64, line: &ChatLine, p: &Palette, draw: impl FnOnce(&mut Ui)) {
    let behind = ui.painter().add(egui::Shape::Noop);
    INLINE.set(None);
    draw(ui);
    if let Some(popup) = INLINE.take()
        && ui.is_visible()
        && !ui.is_sizing_pass()
    {
        let pos = egui::Popup::position_of_id(ui.ctx(), popup).unwrap_or_else(|| ui.min_rect().center());
        let frame = ui.ctx().cumulative_frame_nr();
        ui.data_mut(|d| {
            d.insert_temp(popup.with("inline_chat_frame"), frame);
            if d.get_temp::<Open>(id).is_none_or(|m| m.popup != popup || m.seq != Some(seq)) {
                d.insert_temp(
                    id,
                    Open {
                        target: Arc::new(Target {
                            line: line.clone(),
                            selected_text: None,
                            sender: None,
                        }),
                        pos,
                        seq: Some(seq),
                        popup,
                    },
                );
            }
        });
    }
    if selected(ui.ctx(), id) == Some(seq) {
        ui.painter().set(
            behind,
            egui::epaint::RectShape::filled(ui.min_rect(), ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.18)),
        );
    }
}

pub(super) fn show(ui: &Ui, id: Id, p: &Palette, body: impl FnOnce(&mut Ui, &Target)) {
    let ctx = ui.ctx();
    let Some(menu) = ctx.data(|d| d.get_temp::<Open>(id)) else { return };
    if !is_open(ctx, id) {
        ctx.data_mut(|d| {
            d.remove::<Open>(id);
            d.remove::<u64>(menu.popup.with("inline_chat_frame"));
        });
        return;
    }
    if menu.popup != id {
        // Inline actions belong to their link/preview. Eviction dismisses
        // them rather than leaving an invisible menu holding the viewport.
        if !ui.is_sizing_pass() && ctx.data(|d| d.get_temp::<u64>(menu.popup.with("inline_chat_frame"))) != Some(ctx.cumulative_frame_nr())
        {
            egui::Popup::close_id(ctx, menu.popup);
            ctx.data_mut(|d| {
                d.remove::<Open>(id);
                d.remove::<u64>(menu.popup.with("inline_chat_frame"));
            });
        }
        return;
    }
    super::super::menu::context_menu_at(ui, id, menu.pos, p, |ui| body(ui, &menu.target));
    if !egui::Popup::is_id_open(ctx, id) {
        ctx.data_mut(|d| d.remove::<Open>(id));
    }
}

/// A real right click in the visible transcript, for offline captures.
pub(super) fn demo_mark(ui: &Ui, drawn: &[super::select::Row]) {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("AURORA_DEMO_CONV_LINE_MENU").is_ok_and(|v| v == "1"));
    if !*ENABLED || ui.is_sizing_pass() {
        return;
    }
    let visible: Vec<_> = drawn.iter().filter(|r| ui.clip_rect().contains(r.rect.center())).collect();
    if let Some(row) = visible.get(visible.len() / 2) {
        ui.data_mut(|d| d.insert_temp(Id::unique("chat_line_menu_demo"), row.rect.center()));
    }
}

pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !(700..=702).contains(&frame) {
        return Vec::new();
    }
    let Some(pos) = ctx.data(|d| d.get_temp::<Pos2>(Id::unique("chat_line_menu_demo"))) else {
        return Vec::new();
    };
    if frame == 700 {
        return vec![egui::Event::PointerMoved(pos)];
    }
    vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed: frame == 701,
        modifiers: Default::default(),
    }]
}
