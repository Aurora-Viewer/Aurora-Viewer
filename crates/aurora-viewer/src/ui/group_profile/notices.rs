//! The Notices tab (LLPanelGroupNotices, indra/newview/llpanelgroupnotices.cpp,
//! panel_group_notices.xml, originally LGPL 2.1): the group's notices of
//! the last 14 days (GroupNoticesListRequest), the one chosen read in full
//! (GroupNoticeRequest, answered by an IM_GROUP_NOTICE_REQUESTED instant
//! message) with its attachment to save, and, with the "send notices"
//! ability, a new notice: subject, message and an inventory item that we
//! may copy and transfer (send_group_notice, LLGroupDropTarget).
//!
//! Which notices are unread is Aurora's: those that arrived since the
//! group's list was first seen and were never opened (`world/group_data.rs`).

use super::view::{self, View};
use super::{GroupAction, GroupProfileUi};
use crate::theme::Palette;
use crate::ui::inventory::InvDrag;
use crate::ui::place_details::Ctx;
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::inventory::actions::{COPY, TRANSFER};
use aurora_net::groups::{NoticeSummary, powers};
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// The list's width.
const LIST_W: f32 = 360.0;
/// LLAssetType::AT_LINK, AT_OBJECT.
const AT_LINK: i32 = 24;
const AT_OBJECT: i32 = 6;

/// A notice being written.
#[derive(Default, Clone)]
pub struct Compose {
    pub subject: String,
    pub message: String,
    pub item: Option<Uuid>,
    /// Why the last item dropped was refused.
    pub refused: bool,
}

#[derive(Default)]
pub struct NoticesState {
    pub search: String,
    pub selected: Option<Uuid>,
    pub compose: Option<Compose>,
}

/// The tab shows with the abilities to receive or to send notices
/// (LLPanelGroupNotices::isVisibleByAgent).
pub fn visible(v: &View) -> bool {
    v.member.is_some() && v.can(powers::NOTICES_RECEIVE | powers::NOTICES_SEND)
}

/// The item behind a dragged entry (links resolved), if a group notice may
/// carry it (LLGiveInventory::isInventoryGroupGiveAcceptable: copy and
/// transfer, not a worn attachment).
pub fn attachable(world: &World, id: Uuid) -> Option<Uuid> {
    let inv = &world.inventory;
    let mut item = inv.items.get(&id)?;
    if item.asset_type == AT_LINK {
        item = inv.items.get(&item.asset_id)?;
    }
    let perms = item.owner_mask;
    if perms & COPY == 0 || perms & TRANSFER == 0 {
        return None;
    }
    if item.asset_type == AT_OBJECT && world.worn_attachment_items().contains(&item.id) {
        return None;
    }
    Some(item.id)
}

/// The time of a notice: "10:42" today, "yesterday", or the day and month.
pub fn short_date(timestamp: u32, now: i64) -> String {
    let day = |secs: i64| (secs - 8 * 3600).div_euclid(86_400);
    if timestamp != 0 && day(i64::from(timestamp)) == day(now) {
        let (_, _, _, _, h, m, _) = crate::ui::land::slt_parts(i64::from(timestamp));
        return format!("{h:02}:{m:02}");
    }
    super::home::notice_date(timestamp, now)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The sender as Firestorm shows it: the legacy name, "Resident" dropped.
pub fn sender(from: &str) -> &str {
    from.strip_suffix(" Resident").unwrap_or(from)
}

/// One row of a notice list: the unread mark, the subject, the sender and
/// the attachment, the date. Shared with the Home tab.
pub fn row(ui: &mut egui::Ui, p: &Palette, n: &NoticeSummary, unread: bool, selected: bool, icon: bool, now: i64) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.35));
    }
    let mut x = rect.left() + 8.0;
    if unread {
        ui.painter().circle_filled(Pos2::new(x + 3.5, rect.center().y), 3.5, p.violet);
    }
    x += 7.0 + 10.0;
    if icon {
        let tile = Rect::from_min_size(Pos2::new(x, rect.center().y - 14.0), Vec2::splat(28.0));
        ui.painter().rect_filled(tile, ui.visuals().window_corner_radius, p.raised);
        if let Some(t) = crate::ui::icons::global("speakerphone") {
            ui.painter().image(
                t.id(),
                Rect::from_center_size(tile.center(), Vec2::splat(15.0)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                p.violet_light,
            );
        }
        x = tile.right() + 10.0;
    }
    let date = ui
        .painter()
        .layout_no_wrap(short_date(n.timestamp, now), FontId::proportional(11.0), p.muted_dim);
    let date_w = date.size().x;
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &date, rect.right() - 8.0 - date_w, rect.center().y);
    ui.painter().galley(at, date, p.muted_dim);
    let text_w = (rect.right() - 8.0 - date_w - 10.0 - x).max(20.0);
    widgets::paint_elided(
        ui,
        &n.subject,
        Pos2::new(x, rect.center().y - 8.0),
        text_w,
        FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx())),
        p.ink,
    );
    let from = sender(&n.from);
    let sub_y = rect.center().y + 9.0;
    let from_w = ui
        .painter()
        .layout_no_wrap(from.to_owned(), FontId::proportional(11.5), p.muted)
        .size()
        .x
        .min(text_w);
    widgets::paint_elided(ui, from, Pos2::new(x, sub_y), text_w, FontId::proportional(11.5), p.muted);
    if n.has_attachment && from_w + 30.0 < text_w {
        let dot = ui.painter().layout_no_wrap(" · ".into(), FontId::proportional(11.5), p.muted);
        let dot_w = dot.size().x;
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &dot, x + from_w, sub_y);
        ui.painter().galley(at, dot, p.muted);
        if let Some(t) = crate::ui::icons::global("paperclip") {
            ui.painter().image(
                t.id(),
                Rect::from_center_size(Pos2::new(x + from_w + dot_w + 6.0, sub_y), Vec2::splat(12.0)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                p.muted,
            );
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let body = Rect::from_min_max(
        ui.cursor().min + Vec2::new(4.0, 0.0),
        ui.max_rect().right_bottom() - Vec2::new(4.0, 0.0),
    );
    ui.allocate_rect(body, Sense::hover());
    let list_w = LIST_W.min(body.width() * 0.45);
    let left = Rect::from_min_max(body.min, Pos2::new(body.left() + list_w, body.bottom()));
    let right = Rect::from_min_max(Pos2::new(left.right() + 12.0, body.top()), body.max);
    let mut l = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    l.set_clip_rect(left.intersect(ui.clip_rect()));
    list(st, &mut l, c, v, actions);
    let mut r = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    r.set_clip_rect(right.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("group_notice_view")
        .auto_shrink([false, false])
        .show(&mut r, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            if st.notices.compose.is_some() {
                compose(st, ui, c, v, actions);
            } else {
                reading(st, ui, c, v, actions);
            }
        });
}

/// The search, "New notice", "Refresh", then the notices.
fn list(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let send = v.can(powers::NOTICES_SEND);
        let w = ui.available_width() - if send { 150.0 } else { 40.0 };
        widgets::search_field(ui, &mut st.notices.search, aurora_i18n::tr!("ui-group-notices-search"), w.max(80.0));
        if send
            && widgets::action_button(
                ui,
                p,
                "pencil-filled",
                aurora_i18n::tr!("ui-group-notices-new"),
                ActionKind::Primary,
                Vec2::new(0.0, 26.0),
            )
            .clicked()
        {
            st.notices.compose = Some(Compose::default());
        }
        // onClickRefreshNotices
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("search-refresh"), v.can(powers::NOTICES_RECEIVE)) {
            actions.push(GroupAction::RefreshNotices(v.id));
        }
    });
    ui.add_space(8.0);
    let Some(list) = v.data.and_then(|d| d.notices.as_ref()) else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
        return;
    };
    // updateSelected: the first notice is read when nothing is chosen
    if st.notices.selected.is_none()
        && st.notices.compose.is_none()
        && let Some(first) = list.first()
    {
        st.notices.selected = Some(first.id);
        actions.push(GroupAction::OpenNotice {
            group: v.id,
            notice: first.id,
        });
    }
    let search = st.notices.search.trim().to_lowercase();
    let shown: Vec<&NoticeSummary> = list
        .iter()
        .filter(|n| search.is_empty() || n.subject.to_lowercase().contains(&search) || n.from.to_lowercase().contains(&search))
        .collect();
    let now = now();
    let foot = 24.0;
    egui::ScrollArea::vertical()
        .id_salt("group_notices")
        .max_height((ui.available_height() - foot).max(60.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if list.is_empty() {
                // mNoNoticesStr
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-profile-no-notices"))
                        .size(12.5)
                        .color(p.muted),
                );
            }
            for n in shown {
                let unread = !c.world.group_data.is_read(&v.id, &n.id);
                let selected = st.notices.selected == Some(n.id) && st.notices.compose.is_none();
                if row(ui, p, n, unread, selected, false, now).clicked() {
                    st.notices.selected = Some(n.id);
                    st.notices.compose = None;
                    actions.push(GroupAction::OpenNotice { group: v.id, notice: n.id });
                }
            }
        });
    ui.label(
        egui::RichText::new(aurora_i18n::tr!("ui-group-notices-kept"))
            .size(11.5)
            .color(p.muted_dim),
    );
}

/// The notice chosen, as it came: sender, date, message, attachment.
fn reading(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let shown = v.data.and_then(|d| d.notice.as_ref()).filter(|n| Some(n.id) == st.notices.selected);
    let Some(n) = shown else {
        let text = if st.notices.selected.is_some() {
            aurora_i18n::tr!("common-loading-2")
        } else {
            aurora_i18n::tr!("ui-group-notices-pick")
        };
        view::card(
            ui,
            p,
            "speakerphone",
            p.muted_dim,
            aurora_i18n::tr!("ui-group-profile-notices"),
            None,
            |ui| {
                ui.label(egui::RichText::new(text).size(12.5).color(p.muted));
            },
        );
        return;
    };
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (pic, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                widgets::avatar_disc(ui, p, pic, None, sender(&n.from_name), false);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        egui::RichText::new(&n.subject)
                            .font(FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx())))
                            .color(p.ink),
                    );
                    let when = notice_when(n.timestamp, now());
                    ui.label(
                        egui::RichText::new(format!("{} · {when}", sender(&n.from_name)))
                            .size(12.0)
                            .color(p.muted),
                    );
                });
            });
            ui.add_space(8.0);
            for line in n.message.lines() {
                if line.trim().is_empty() {
                    ui.add_space(6.0);
                } else {
                    crate::ui::chat::chat_text(ui, p, c.emoji, c.world, c.want_names, line, 13.0, p.ink, false);
                }
            }
            if let Some((name, asset_type)) = &n.attachment {
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(p.bar)
                    .corner_radius(ui.visuals().widgets.inactive.corner_radius)
                    .inner_margin(egui::Margin::symmetric(9, 7))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            widgets::glyph(ui, attachment_icon(*asset_type), p.amber, 17.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(
                                    egui::RichText::new(name)
                                        .font(FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())))
                                        .color(p.ink),
                                );
                                ui.label(egui::RichText::new(asset_label(*asset_type)).size(11.5).color(p.muted));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let (icon, label) = if n.saved {
                                    ("check", aurora_i18n::tr!("ui-group-notices-saved"))
                                } else {
                                    ("download-filled", aurora_i18n::tr!("ui-group-notices-save"))
                                };
                                if widgets::action_button_if(ui, p, icon, label, ActionKind::Flat, Vec2::new(0.0, 26.0), !n.saved).clicked()
                                {
                                    actions.push(GroupAction::SaveAttachment(v.id));
                                }
                            });
                        });
                    });
            }
        });
}

/// "today 10:42", "yesterday", or the date.
fn notice_when(timestamp: u32, now: i64) -> String {
    let short = short_date(timestamp, now);
    if short.contains(':') {
        format!("{} {short}", aurora_i18n::tr!("ui-chat-today").to_lowercase())
    } else {
        short
    }
}

/// The icon of an attachment by asset type (the inventory's).
pub(crate) fn attachment_icon(asset_type: u8) -> &'static str {
    match asset_type {
        0 => "photo-filled",
        1 => "volume",
        3 => "map-pin-filled",
        7 => "file-text-filled",
        10 => "code",
        20 => "run",
        21 => "hand-finger",
        _ => "package",
    }
}

/// What an attachment is, in words.
pub(crate) fn asset_label(asset_type: u8) -> &'static str {
    match asset_type {
        0 => aurora_i18n::tr!("common-texture"),
        1 => aurora_i18n::tr!("common-sound"),
        3 => aurora_i18n::tr!("common-landmark"),
        5 => aurora_i18n::tr!("common-clothing"),
        6 => aurora_i18n::tr!("common-object"),
        _ => aurora_i18n::tr!("ui-group-notices-attachment"),
    }
}

/// A new notice: subject, message, an attachment dropped from the
/// inventory; sent to every member.
fn compose(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let Some(draft) = st.notices.compose.as_mut() else {
        return;
    };
    let mut cancel = false;
    let mut send = false;
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .stroke(egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.35)))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                widgets::glyph(ui, "pencil-filled", p.muted_dim, 13.0);
                let mut format = crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, p.muted_dim);
                format.extra_letter_spacing = 0.4;
                let mut job = egui::text::LayoutJob::default();
                job.append(&aurora_i18n::tr!("ui-group-notices-new").to_uppercase(), 0.0, format);
                ui.label(job);
            });
            ui.add(
                egui::TextEdit::singleline(&mut draft.subject)
                    .hint_text(aurora_i18n::tr!("ui-group-notices-subject"))
                    .char_limit(63)
                    .desired_width(f32::INFINITY),
            );
            let mut message = egui::TextEdit::multiline(&mut draft.message)
                .hint_text(aurora_i18n::tr!("ui-group-notices-message"))
                .char_limit(511)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .show(ui);
            // spell checked, as panel_group_notices.xml
            crate::ui::spelling::field(ui, p, &mut message, &mut draft.message, false);
            // the attachment: dropped from the inventory
            let (zone, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::hover());
            let hovered = resp.dnd_hover_payload::<InvDrag>().map(|d| d.0);
            let ok = hovered.map(|id| attachable(c.world, id).is_some());
            let stroke = match ok {
                Some(true) => p.violet_light,
                Some(false) => p.danger,
                None => p.raised,
            };
            ui.painter().rect_stroke(
                zone,
                ui.visuals().window_corner_radius,
                egui::Stroke::new(1.5, stroke),
                egui::StrokeKind::Inside,
            );
            if let Some(d) = resp.dnd_release_payload::<InvDrag>() {
                match attachable(c.world, d.0) {
                    Some(item) => {
                        draft.item = Some(item);
                        draft.refused = false;
                    }
                    None => draft.refused = true,
                }
            }
            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(zone.shrink2(Vec2::new(9.0, 4.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            inner.spacing_mut().item_spacing.x = 6.0;
            widgets::glyph(&mut inner, "paperclip", p.muted, 14.0);
            match draft.item.and_then(|id| c.world.inventory.items.get(&id)) {
                Some(item) => {
                    inner.label(egui::RichText::new(&item.name).size(12.5).color(p.ink));
                    inner.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("ui-group-notices-remove-attachment"), true) {
                            draft.item = None;
                        }
                    });
                }
                None => {
                    let text = if draft.refused {
                        aurora_i18n::tr!("ui-group-notices-not-attachable")
                    } else {
                        aurora_i18n::tr!("ui-group-notices-drop")
                    };
                    inner.label(
                        egui::RichText::new(text)
                            .size(12.0)
                            .color(if draft.refused { p.danger } else { p.muted_dim }),
                    );
                }
            }
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let count = v.member_count().unwrap_or(0);
                    let label = aurora_i18n::tr!("ui-group-notices-send", count = count, shown = aurora_i18n::integer(count));
                    // MustSpecifyGroupNoticeSubject
                    let ready = !draft.subject.trim().is_empty();
                    if widgets::action_button_if(ui, p, "send-filled", &label, ActionKind::Primary, Vec2::new(0.0, 28.0), ready).clicked() {
                        send = true;
                    }
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, Vec2::new(0.0, 28.0))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });
        });
    if send && let Some(d) = st.notices.compose.take() {
        let item = d.item.and_then(|id| c.world.inventory.items.get(&id)).map(|it| (it.id, it.owner));
        actions.push(GroupAction::SendNotice {
            group: v.id,
            subject: d.subject,
            message: d.message,
            item,
        });
    } else if cancel {
        st.notices.compose = None;
    }
}
