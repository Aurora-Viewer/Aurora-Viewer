//! A transcript owns its menu independently of the virtualized row that
//! opened it. Keep only that target, as LLChatHistoryHeader's avatar menu
//! does (indra/newview/llchathistory.cpp), while messages keep updating.

use crate::theme::Palette;
use crate::world::ChatLine;
use egui::{Id, Pos2, Response, Ui};
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct Target {
    pub line: ChatLine,
    /// None: the message's Copy/Quote menu; Some: its sender's menu.
    pub sender: Option<Uuid>,
}

#[derive(Clone)]
struct Open {
    target: Arc<Target>,
    pos: Pos2,
}

fn open(response: &Response, id: Id, line: &ChatLine, sender: Option<Uuid>) {
    if let Some(pos) = response.interact_pointer_pos() {
        response.ctx.data_mut(|d| {
            d.insert_temp(
                id,
                Open {
                    target: Arc::new(Target {
                        line: line.clone(),
                        sender,
                    }),
                    pos,
                },
            );
        });
        egui::Popup::open_id(&response.ctx, id);
    }
}

pub(super) fn sender(response: &Response, id: Id, avatar: Uuid, line: &ChatLine) {
    if response.secondary_clicked() {
        open(response, id, line, Some(avatar));
    }
}

pub(super) fn message<'l>(response: &Response, id: Id, drawn: &[super::select::Row], line_of: impl Fn(u64) -> Option<&'l ChatLine>) {
    if response.secondary_clicked()
        && let Some(line) = response
            .interact_pointer_pos()
            .and_then(|pos| drawn.iter().find(|row| row.rect.contains(pos)))
            .and_then(|row| line_of(row.seq))
    {
        open(response, id, line, None);
    }
}

pub(super) fn show(ui: &Ui, id: Id, p: &Palette, body: impl FnOnce(&mut Ui, &Target)) {
    let ctx = ui.ctx();
    if !egui::Popup::is_id_open(ctx, id) {
        ctx.data_mut(|d| d.remove::<Open>(id));
        return;
    }
    let Some(menu) = ctx.data(|d| d.get_temp::<Open>(id)) else { return };
    super::super::menu::context_menu_at(ui, id, menu.pos, p, |ui| body(ui, &menu.target));
    if !egui::Popup::is_id_open(ctx, id) {
        ctx.data_mut(|d| d.remove::<Open>(id));
    }
}
