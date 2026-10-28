//! The Home tab: the charter and the latest notices on the left; the
//! members online and, for a member, "Me in this group" (the settings of
//! LLPanelGroupGeneral: active group, title, notices, chat, listing in our
//! profile) on the right.

use super::members::{self, Show};
use super::notices;
use super::view::{self, View};
use super::{Env, GroupAction, GroupProfileUi, Tab};
use crate::theme::Palette;
use crate::ui::place_details::Ctx;
use crate::ui::widgets;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

/// Notices shown on the tab, and members online at most.
const NOTICES: usize = 3;
const ONLINE: usize = 8;
/// Width of the right column.
const SIDE_W: f32 = 290.0;

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &View, actions: &mut Vec<GroupAction>) {
    egui::ScrollArea::vertical()
        .id_salt("group_profile_home")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width() - 8.0;
            let side_w = SIDE_W.min(width * 0.4);
            let main_w = width - side_w - 12.0;
            ui.horizontal_top(|ui| {
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.allocate_ui_with_layout(Vec2::new(main_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(main_w);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    charter(ui, c, v);
                    if super::notices::visible(v) {
                        notices(st, ui, c, v, actions);
                    }
                });
                ui.allocate_ui_with_layout(Vec2::new(side_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(side_w);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    online(st, ui, c, env, v);
                    if v.member.is_some() {
                        me(ui, c, v, actions);
                    }
                });
            });
        });
}

fn charter(ui: &mut egui::Ui, c: &mut Ctx, v: &View) {
    let p = c.p;
    view::card(
        ui,
        p,
        "file-text-filled",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-profile-charter"),
        None,
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let Some(g) = v.profile else {
                ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
                return;
            };
            if g.charter.trim().is_empty() {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-profile-no-charter"))
                        .size(12.5)
                        .color(p.muted),
                );
            }
            for line in g.charter.lines() {
                if line.trim().is_empty() {
                    ui.add_space(6.0);
                } else {
                    crate::ui::chat::chat_text(ui, p, c.emoji, c.world, c.want_names, line, 13.0, p.ink, false);
                }
            }
        },
    );
}

/// "today", "yesterday", or the day and the month (in SLT, as the server
/// dates notices).
pub fn notice_date(timestamp: u32, now: i64) -> String {
    let day = |secs: i64| aurora_net::profile::civil_from_days((secs - 8 * 3600).div_euclid(86_400));
    let (y, m, d) = day(i64::from(timestamp));
    let today = day(now);
    let days = aurora_net::profile::days_from_civil(today.0, today.1, today.2) - aurora_net::profile::days_from_civil(y, m, d);
    match days {
        0 => aurora_i18n::tr!("ui-chat-today").to_lowercase(),
        1 => aurora_i18n::tr!("ui-chat-yesterday").to_lowercase(),
        _ => crate::ui::land::day_month((y != today.0).then_some(y), m as usize, i64::from(d)),
    }
}

/// The three latest notices (GroupNoticesListReply), each opening in the
/// Notices tab.
fn notices(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let (_, all) = view::card(
        ui,
        p,
        "speakerphone",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-profile-latest-notices"),
        Some(aurora_i18n::tr!("ui-group-profile-all-notices")),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let Some(list) = v.data.and_then(|d| d.notices.as_ref()) else {
                ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
                return;
            };
            if list.is_empty() {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-profile-no-notices"))
                        .size(12.5)
                        .color(p.muted),
                );
                return;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            for n in list.iter().take(NOTICES) {
                let unread = !c.world.group_data.is_read(&v.id, &n.id);
                if notices::row(ui, p, n, unread, false, true, now).clicked() {
                    st.tab = Tab::Notices;
                    st.notices.selected = Some(n.id);
                    st.notices.compose = None;
                    actions.push(GroupAction::OpenNotice { group: v.id, notice: n.id });
                }
            }
        },
    );
    if all {
        st.tab = Tab::Notices;
    }
}

/// The members online now, owners first, then the officers, then by name.
fn online(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &View) {
    let p = c.p;
    let count = v.online_count();
    let title = match count {
        Some(n) => aurora_i18n::tr!("ui-group-profile-online-now", count = aurora_i18n::integer(n as i64)),
        None => aurora_i18n::tr!("ui-group-profile-online-now-title").to_owned(),
    };
    let (_, clicked) = view::card(
        ui,
        p,
        "circle-filled",
        p.success,
        &title,
        Some(aurora_i18n::tr!("ui-group-profile-all-members")),
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let Some(d) = v.data.filter(|d| d.members_complete) else {
                ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
                return;
            };
            let world = c.world;
            let mut list: Vec<_> = d
                .online()
                .map(|m| (m, members::chip_of(v, m), world.social.display_name_of(&m.id)))
                .collect();
            if list.is_empty() {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-profile-nobody-online"))
                        .size(12.5)
                        .color(p.muted),
                );
                return;
            }
            list.sort_by_cached_key(|(_, chip, name)| (*chip as u8, name.to_lowercase()));
            for (m, chip, name) in list.into_iter().take(ONLINE) {
                if name == "…" {
                    c.want_names.insert(m.id);
                }
                let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
                let pic = Rect::from_min_size(Pos2::new(row.left(), row.center().y - 15.0), Vec2::splat(30.0));
                let pics = env.avatar_pics.get(&m.id);
                if pics.is_none() {
                    st.want_pic(m.id, env.now);
                }
                widgets::avatar_disc(ui, p, pic, pics, &name, false);
                let status = match crate::ui::presence::of(world, &m.id) {
                    crate::ui::presence::Status::Away => crate::ui::presence::Status::Away,
                    crate::ui::presence::Status::Busy => crate::ui::presence::Status::Busy,
                    _ => crate::ui::presence::Status::Online,
                };
                crate::ui::presence::paint_dot(ui.painter(), p, pic, status, p.field);
                // the title as the chip, at the right
                let title = if m.title.is_empty() {
                    members::role_name("Everyone")
                } else {
                    m.title.clone()
                };
                let chip_w = {
                    let font = FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx()));
                    ui.painter().layout_no_wrap(title.clone(), font, p.muted).size().x + 12.0
                };
                let chip_rect = Rect::from_min_max(Pos2::new(row.right() - chip_w, row.top()), row.right_bottom());
                let mut chip_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(chip_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                view::role_chip(&mut chip_ui, p, &title, chip);
                widgets::paint_elided(
                    ui,
                    &name,
                    Pos2::new(pic.right() + 8.0, row.center().y),
                    (chip_rect.left() - 8.0 - pic.right() - 8.0).max(20.0),
                    FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                    p.ink,
                );
                if resp.double_clicked() {
                    crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::Profile(m.id));
                }
                crate::ui::menu::context_menu(&resp, p, |ui| {
                    crate::ui::context::avatar_list_menu(ui, p, world, m.id, crate::ui::context::AvatarList::Name);
                });
            }
        },
    );
    if clicked {
        st.tab = Tab::Members;
        st.members.show = Show::Online;
    }
}

/// One setting of "Me in this group": an icon, its name, its control.
fn setting_row(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        widgets::glyph(ui, icon, p.muted, 15.0);
        ui.label(egui::RichText::new(label).size(12.5).color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

/// Our settings in the group (LLPanelGroupGeneral's "My group settings"
/// and LLGroupActions::activate, exoGroupMuteList for the chat).
fn me(ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let Some(m) = v.member else { return };
    view::card(
        ui,
        p,
        "user-cog",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-profile-me"),
        None,
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            setting_row(ui, p, "star-filled", aurora_i18n::tr!("ui-group-profile-active"), |ui| {
                let mut on = v.active;
                if widgets::switch(ui, p, &mut on).changed() {
                    actions.push(GroupAction::Activate(if on { v.id } else { uuid::Uuid::nil() }));
                }
            });
            setting_row(ui, p, "tag-filled", aurora_i18n::tr!("ui-group-profile-title-shown"), |ui| {
                let titles = v.data.map(|d| d.titles.as_slice()).unwrap_or_default();
                let current = titles.iter().find(|t| t.selected);
                let text = current.map_or_else(|| v.my_title().unwrap_or_default(), |t| t.title.clone());
                ui.add_enabled_ui(!titles.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt(("group_title", v.id))
                        .width(130.0)
                        .selected_text(text)
                        .show_ui(ui, |ui| {
                            for t in titles {
                                if ui.selectable_label(t.selected, &t.title).clicked() && !t.selected {
                                    actions.push(GroupAction::SetTitle { group: v.id, role: t.role });
                                }
                            }
                        });
                });
            });
            setting_row(ui, p, "bell-filled", aurora_i18n::tr!("ui-group-profile-receive-notices"), |ui| {
                let mut on = m.accept_notices;
                if widgets::switch(ui, p, &mut on).changed() {
                    actions.push(GroupAction::SetFlags {
                        group: v.id,
                        accept_notices: on,
                        list_in_profile: m.list_in_profile,
                    });
                }
            });
            setting_row(ui, p, "messages-filled", aurora_i18n::tr!("ui-group-profile-receive-chat"), |ui| {
                let blocked = c.world.mutes.group_chat_muted(&v.id);
                let mut on = !blocked;
                if widgets::switch(ui, p, &mut on).changed() {
                    crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::GroupChatBlocked(v.id, !on));
                }
            });
            setting_row(ui, p, "id-filled", aurora_i18n::tr!("ui-group-profile-show-in-profile"), |ui| {
                let mut on = m.list_in_profile;
                if widgets::switch(ui, p, &mut on).changed() {
                    actions.push(GroupAction::SetFlags {
                        group: v.id,
                        accept_notices: m.accept_notices,
                        list_in_profile: on,
                    });
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_dates_read_today_yesterday_then_the_day() {
        // 2026-10-14 12:00 SLT
        let now = aurora_net::profile::days_from_civil(2026, 10, 14) * 86_400 + 20 * 3600;
        let at = |y, m, d| (aurora_net::profile::days_from_civil(y, m, d) * 86_400 + 20 * 3600) as u32;
        assert_eq!(notice_date(at(2026, 10, 14), now), aurora_i18n::tr!("ui-chat-today").to_lowercase());
        assert_eq!(
            notice_date(at(2026, 10, 13), now),
            aurora_i18n::tr!("ui-chat-yesterday").to_lowercase()
        );
        let oct = crate::ui::land::months()[9];
        assert_eq!(notice_date(at(2026, 10, 3), now), format!("3 {oct}"));
        assert_eq!(notice_date(at(2025, 10, 3), now), format!("3 {oct} 2025"));
    }
}
