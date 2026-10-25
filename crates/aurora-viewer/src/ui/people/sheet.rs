//! The sheet of a person at the right of People (the mock-up's "fiche"):
//! who they are, the actions of the avatar menu as buttons, the rights of a
//! friendship (the ones we grant, changed with a switch as the contact
//! list's boxes change them, applyRightsToFriends; the ones we receive),
//! what the radar knows of them, then the map, the payment and the
//! friendship at the bottom. "…" holds the rest of the avatar menu
//! (menu_people_nearby.xml): nothing of Firestorm's menus is lost.

use super::nearby::{Band, distance_text, seen_text};
use super::radar::{Age, Payment, RadarData, Row};
use super::{PeopleAction, PeopleUi};
use crate::theme::Palette;
use crate::ui::contacts::SetEdit;
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::widgets::{self, ActionKind};
use crate::ui::{menu, presence};
use crate::world::World;
use crate::world::social::{Friend, rights};
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// Height of the buttons, of the bar of buttons at the bottom, and of a line
/// of a section.
pub(super) const BUTTON_H: f32 = 28.0;
const LINE_H: f32 = 25.0;
/// Room around the content.
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 10.0;
/// The window's margin the sheet's face covers: 10 points, its outline
/// left alone.
const SHEET_BLEED: f32 = 9.0;

/// The rights we grant, as the switches say them.
const GRANTS: [(i32, &str, &str); 3] = [
    (rights::ONLINE_STATUS, "eye-filled", "ui-profile-see-when-i-am-online"),
    (rights::MAP_LOCATION, "map-pin-filled", "people-sheet-grant-map"),
    (
        rights::MODIFY_OBJECTS,
        "pencil-filled",
        "ui-profile-modify-delete-or-take-my-objects",
    ),
];

/// The same rights, received.
const RECEIVED: [(i32, &str, &str); 3] = [
    (rights::ONLINE_STATUS, "eye-filled", "people-sheet-has-online"),
    (rights::MAP_LOCATION, "map-pin-filled", "people-sheet-has-map"),
    (rights::MODIFY_OBJECTS, "pencil-filled", "ui-people-i-can-modify-their-objects"),
];

fn semibold(ui: &egui::Ui, size: f32, color: Color32) -> egui::TextFormat {
    crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), size, color)
}

/// The face of a sheet over the whole of `ui` (a quieter face than the
/// list's, out to the window's right and bottom edges: over the frame's
/// margins, inside its outline); returns its content and its bottom bar.
pub(super) fn frame(ui: &egui::Ui, p: &Palette) -> (Rect, Rect) {
    let rect = ui.max_rect();
    let face = Rect::from_min_max(rect.min, rect.max + Vec2::splat(SHEET_BLEED));
    let painter = ui.painter().with_clip_rect(face);
    painter.rect_filled(face, 0.0, p.bar.gamma_multiply(0.5));
    painter.vline(rect.left(), face.y_range(), widgets::divider_stroke(p));
    let inner = rect.shrink2(Vec2::new(PAD_X, PAD_Y));
    let top = inner.bottom() - BUTTON_H;
    (
        Rect::from_min_max(inner.min, Pos2::new(inner.right(), top - 8.0)),
        Rect::from_min_max(Pos2::new(inner.left(), top), inner.max),
    )
}

/// Height of a sheet's head (picture, names, cross), and the gap under it.
const HEAD_H: f32 = 44.0;
const HEAD_GAP: f32 = 10.0;

/// A sheet's content split into its fixed head and what scrolls below.
pub(super) fn split_head(content: Rect) -> (Rect, Rect) {
    let top = Rect::from_min_size(content.min, Vec2::new(content.width(), HEAD_H));
    let rest = Rect::from_min_max(
        Pos2::new(content.left(), (top.bottom() + HEAD_GAP).min(content.bottom())),
        content.max,
    );
    (top, rest)
}

/// A top-down child of `ui` over `rect`.
pub(super) fn child(ui: &mut egui::Ui, rect: Rect, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Ui {
    ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    )
}

/// The sheet of `id` over the whole of `ui`, with what the radar knows of
/// them when they are around (in Near you, it says when they left). Returns
/// true when its cross was clicked.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    data: &RadarData,
    st: &mut PeopleUi,
    id: Uuid,
    nearby_tab: bool,
    closable: bool,
    actions: &mut Vec<PeopleAction>,
) -> bool {
    let found = super::radar::rows(world, data, "").into_iter().find(|r| r.id == id);
    let row = found.as_ref();
    let friend = world.social.friends.iter().find(|f| f.id == id).cloned();
    let (content, bottom) = frame(ui, p);
    if !data.pics.contains_key(&id) {
        st.want_pic(id, ui.input(|i| i.time));
    }
    // The head stays put above what scrolls.
    let (top, rest) = split_head(content);
    let closed = head(&mut child(ui, top, ("people-sheet-head", id)), p, world, data, id, row, closable);
    egui::ScrollArea::vertical()
        .id_salt(("people-sheet-scroll", id))
        .auto_shrink([false, false])
        .show(&mut child(ui, rest, ("people-sheet-body", id)), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            buttons(ui, p, world, st, id, friend.as_ref(), actions);
            if let Some(f) = &friend {
                let name = world.social.display_name_of(&id);
                ui.add_space(12.0);
                granted(ui, p, f, &name, actions);
                ui.add_space(10.0);
                received(ui, p, f, &name);
            }
            if id != world.agent_id {
                ui.add_space(10.0);
                nickname_and_sets(ui, p, world, st, id);
            }
            // what the radar knows, in Near you (the other tabs' sheets
            // keep to the person, as the mock-up's)
            if nearby_tab {
                ui.add_space(10.0);
                match row {
                    Some(row) => nearby(ui, p, data, row),
                    None => {
                        ui.label(egui::RichText::new(aurora_i18n::tr!("people-sheet-gone")).size(12.0).color(p.muted));
                    }
                }
            }
        });
    let mut foot = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("people-sheet-foot", id))
            .max_rect(bottom)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    footer(&mut foot, p, world, id, friend.as_ref());
    closed
}

/// The picture with its status dot, the names, and the cross.
fn head(ui: &mut egui::Ui, p: &Palette, world: &World, data: &RadarData, id: Uuid, row: Option<&Row>, closable: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    let names = &world.social.avatar_names;
    let display = world.social.display_name_of(&id);
    let disc = Rect::from_center_size(Pos2::new(rect.left() + 20.0, rect.center().y), Vec2::splat(40.0));
    widgets::avatar_disc(ui, p, disc, data.pics.get(&id), &display, row.is_some_and(|r| r.muted));
    let status = presence::of(world, &id);
    if status != presence::Status::Unknown {
        presence::paint_dot(ui.painter(), p, disc, status, p.panel);
    }
    // the cross at the right, the names between
    let (right, closed) = if closable {
        let cross = Rect::from_center_size(Pos2::new(rect.right() - 10.0, rect.center().y), Vec2::splat(20.0));
        let resp = ui.interact(cross, ui.scope_id().with(("people-sheet-close", id)), Sense::click());
        if resp.hovered() {
            ui.painter()
                .rect_filled(cross, ui.visuals().widgets.inactive.corner_radius, p.raised);
        }
        super::radar::icon(ui, "x", cross, 14.0, if resp.hovered() { p.ink } else { p.muted });
        (cross.left() - 8.0, resp.on_hover_text(aurora_i18n::tr!("common-close")).clicked())
    } else {
        (rect.right(), false)
    };
    let left = disc.right() + 10.0;
    let width = (right - left).max(20.0);
    // "Legacy Name · account name" under the display name: who they are
    // whatever the name preferences
    let sub = names.get(&id).map(|n| {
        let legacy = crate::world::social::format_name(&n.legacy_first, &n.legacy_last);
        if n.is_default || legacy.is_empty() {
            n.username.clone()
        } else {
            format!("{legacy} · {}", n.username)
        }
    });
    let mut job = egui::text::LayoutJob::single_section(display, semibold(ui, 15.0, p.ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let name_g = ui.painter().layout_job(job);
    let mut job = egui::text::LayoutJob::single_section(
        sub.unwrap_or_default(),
        egui::TextFormat::simple(FontId::proportional(12.0), p.muted),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let sub_g = ui.painter().layout_job(job);
    let top = rect.center().y - (name_g.size().y + sub_g.size().y) / 2.0;
    let name_h = name_g.size().y;
    ui.painter().galley(Pos2::new(left, top), name_g, p.ink);
    ui.painter().galley(Pos2::new(left, top + name_h), sub_g, p.muted);
    closed
}

/// The permanent inline panel keeps its place when no friend matches.
pub(super) fn empty(ui: &mut egui::Ui, p: &Palette) {
    let (content, _) = frame(ui, p);
    let center = content.center();
    let icon = Rect::from_center_size(center - Vec2::new(0.0, 22.0), Vec2::splat(32.0));
    super::radar::icon(ui, "user-filled", icon, 32.0, p.muted_dim);
    let mut job = egui::text::LayoutJob::single_section(
        aurora_i18n::tr!("people-contact-details-empty").to_owned(),
        egui::TextFormat::simple(FontId::proportional(13.0), p.muted),
    );
    job.wrap.max_width = content.width();
    let galley = ui.painter().layout_job(job);
    widgets::paint_ink_centered(ui.painter(), galley, center + Vec2::new(0.0, 16.0), p.muted);
}

/// The actions of the avatar menu people use the most, the rest under "…".
fn buttons(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut PeopleUi,
    id: Uuid,
    friend: Option<&Friend>,
    actions: &mut Vec<PeopleAction>,
) {
    let online = friend.is_none_or(|f| f.online);
    let own = id == world.agent_id;
    let size = Vec2::new(0.0, BUTTON_H);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
        if widgets::action_button(
            ui,
            p,
            "message-circle-filled",
            aurora_i18n::tr!("people-sheet-im"),
            ActionKind::Primary,
            size,
        )
        .clicked()
        {
            actions.push(PeopleAction::OpenIm(id));
        }
        let call = widgets::action_button_if(
            ui,
            p,
            "phone-filled",
            aurora_i18n::tr!("ui-profile-aurora-call"),
            ActionKind::Flat,
            size,
            world.calls.voice_enabled && !own,
        );
        if call.clicked() {
            context::request(ui.ctx(), CtxAction::Call(id));
        }
        if widgets::action_button(ui, p, "user-filled", aurora_i18n::tr!("common-profile"), ActionKind::Flat, size).clicked() {
            actions.push(PeopleAction::Profile(id));
        }
        let tp = widgets::action_button_if(
            ui,
            p,
            "send-filled",
            aurora_i18n::tr!("ui-profile-aurora-offer-tp"),
            ActionKind::Flat,
            size,
            online && !own,
        );
        if tp.clicked() {
            actions.push(PeopleAction::OfferTeleport(id));
        }
        let more = widgets::action_button(ui, p, "dots-filled", "", ActionKind::Flat, Vec2::new(BUTTON_H + 4.0, BUTTON_H))
            .on_hover_text(aurora_i18n::tr!("ui-chat-more"));
        let kind = if friend.is_some() { AvatarList::Friend } else { AvatarList::Nearby };
        let open = menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            context::avatar_list_menu(ui, p, world, id, kind);
            if !own {
                // the contact sets' last button (FSPanelContactSets)
                menu::separator(ui, p);
                if menu::item_if(
                    ui,
                    p,
                    "id-off",
                    aurora_i18n::tr!("ui-contacts-remove-display-name"),
                    !world.contact_sets.has_display_name_removed(&id),
                ) {
                    context::request(ui.ctx(), CtxAction::ContactSet(SetEdit::RemoveDisplayName(vec![id])));
                }
            }
        });
        // the sheet's own menu is not a row's
        if open {
            st.row_menu = None;
        }
    });
}

/// The title of a part of the sheet, in small capitals.
pub(super) fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .font(FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())))
            .color(p.muted_dim),
    );
    ui.add_space(4.0);
}

/// One line of a part: its icon and its label at the left, `control` (its
/// width) at the right edge.
pub(super) fn line(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, control_w: f32, control: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let text_w = (width - 26.0 - control_w - 8.0).max(40.0);
    let galley = ui.painter().layout(label.to_owned(), FontId::proportional(12.5), p.ink, text_w);
    let h = (galley.size().y + 6.0).max(LINE_H);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
    super::radar::icon(
        ui,
        icon,
        Rect::from_center_size(Pos2::new(rect.left() + 9.0, rect.center().y), Vec2::splat(18.0)),
        15.0,
        p.muted,
    );
    crate::ui::widgets::paint_row_text(ui.painter(), galley, rect.left() + 26.0, rect.center().y, p.ink);
    let slot = Rect::from_min_max(Pos2::new(rect.right() - control_w, rect.top()), rect.max);
    control(
        &mut ui.new_child(
            egui::UiBuilder::new()
                .max_rect(slot)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        ),
    );
}

/// What we grant the friend, a switch each. The map goes with the online
/// status (applyRightsToFriends); the right to modify our objects is asked
/// first (GrantModifyRights / RevokeModifyRights).
fn granted(ui: &mut egui::Ui, p: &Palette, f: &Friend, name: &str, actions: &mut Vec<PeopleAction>) {
    section(ui, p, &aurora_i18n::tr!("people-sheet-i-grant", name = name.to_owned()));
    for (bit, icon, key) in GRANTS {
        let mut on = f.rights_given & bit != 0;
        line(ui, p, icon, aurora_i18n::text(key), widgets::SWITCH_SIZE.x, |ui| {
            if widgets::switch(ui, p, &mut on).changed() {
                let rights = crate::ui::contacts::rights_after(f.rights_given, bit, on);
                if bit == rights::MODIFY_OBJECTS {
                    actions.push(PeopleAction::ModifyRights {
                        friend: f.id,
                        rights,
                        grant: on,
                    });
                } else {
                    context::request(ui.ctx(), CtxAction::GrantRights { friend: f.id, rights });
                }
            }
        });
    }
}

/// What the friend grants us: read only.
fn received(ui: &mut egui::Ui, p: &Palette, f: &Friend, name: &str) {
    section(ui, p, &aurora_i18n::tr!("people-sheet-they-grant", name = name.to_owned()));
    for (bit, icon, key) in RECEIVED {
        let on = f.rights_has & bit != 0;
        let (text, format) = if on {
            (aurora_i18n::text("common-yes").to_lowercase(), semibold(ui, 11.5, p.success))
        } else {
            (
                aurora_i18n::text("common-no").to_lowercase(),
                egui::TextFormat::simple(FontId::proportional(11.5), p.muted_dim),
            )
        };
        let galley = ui.painter().layout_job(egui::text::LayoutJob::single_section(text, format));
        let w = galley.size().x;
        line(ui, p, icon, aurora_i18n::text(key), w, |ui| {
            ui.label(galley);
        });
    }
}

/// "Nickname and contact sets": the nickname in a field (Enter or leaving
/// it sets it, empty or the cross takes it away: setPseudonym /
/// clearPseudonym), then the sets they are in as chips (the cross takes
/// them out, confirmed as in the contact sets' list) and "+ Set" to add
/// them to another (LLAvatarActions::addToContactSet).
fn nickname_and_sets(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut PeopleUi, id: Uuid) {
    use crate::world::contact_sets::DN_REMOVED;
    section(ui, p, aurora_i18n::tr!("people-sheet-nickname-sets"));
    let current = world
        .contact_sets
        .pseudonym(&id)
        .filter(|a| *a != DN_REMOVED)
        .unwrap_or_default()
        .to_owned();
    if st.nick.as_ref().is_none_or(|(who, _)| *who != id) {
        st.nick = Some((id, current.clone()));
    }
    let Some((_, text)) = st.nick.as_mut() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let cross_w = BUTTON_H + 2.0;
        let field_w = ui.available_width() - cross_w - 6.0;
        let field = egui::Frame::new()
            .fill(p.field)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(field_w - 16.0);
                ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(egui::Frame::NONE.inner_margin(crate::ui::widgets::field_text_margin(
                            ui.ctx(),
                            &egui::TextStyle::Body.resolve(ui.style()),
                        )))
                        .hint_text(aurora_i18n::tr!("people-sheet-nickname-hint"))
                        .desired_width(f32::INFINITY),
                )
            })
            .inner;
        if field.lost_focus() && text.trim() != current {
            context::request(
                ui.ctx(),
                CtxAction::ContactSet(SetEdit::Nickname {
                    ids: vec![id],
                    alias: text.trim().to_owned(),
                }),
            );
        }
        let clear = widgets::action_button_if(ui, p, "x", "", ActionKind::Flat, Vec2::new(cross_w, BUTTON_H), !current.is_empty())
            .on_hover_text(aurora_i18n::tr!("ui-contacts-remove-nickname"));
        if clear.clicked() {
            text.clear();
            context::request(ui.ctx(), CtxAction::ContactSet(SetEdit::ClearNickname(vec![id])));
        }
    });
    ui.add_space(6.0);
    let mine = world.contact_sets.friend_sets(&id);
    let others: Vec<String> = world.contact_sets.set_names().into_iter().filter(|s| !mine.contains(s)).collect();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
        for set in &mine {
            let color = world
                .contact_sets
                .set(set)
                .map_or(p.muted, |s| crate::ui::contacts::color32(s.color));
            let (_, closed) = widgets::dot_pill(ui, p, color, set, None, false, true);
            if closed {
                st.asks.push(super::Ask::RemoveFromSet(set.clone(), vec![id]));
            }
        }
        let add = widgets::filter_pill(ui, p, "plus", aurora_i18n::tr!("people-sheet-add-set"), false);
        menu::dropdown(&add, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            if others.is_empty() {
                menu::item_if(ui, p, "users-group", aurora_i18n::tr!("ui-contacts-no-other-contact-sets"), false);
            }
            for set in &others {
                if menu::item(ui, p, "users-group", set) {
                    context::request(
                        ui.ctx(),
                        CtxAction::ContactSet(SetEdit::Add {
                            set: set.clone(),
                            ids: vec![id],
                        }),
                    );
                }
            }
            menu::separator(ui, p);
            if menu::item(ui, p, "plus", aurora_i18n::tr!("ui-contacts-add-a-new-contact-set")) {
                st.asks.push(super::Ask::NewSet);
            }
        });
    });
}

/// What the radar knows of them: range, time here, voice, age, payment
/// info and our note (the columns of Firestorm's table).
fn nearby(ui: &mut egui::Ui, p: &Palette, data: &RadarData, row: &Row) {
    section(ui, p, aurora_i18n::tr!("ui-people-near-you"));
    let value = |ui: &mut egui::Ui, text: String, color: Color32| {
        ui.label(egui::RichText::new(text).size(12.0).color(color));
    };
    let band = Band::of(row.range);
    let range = match row.range {
        Some(_) => format!("{} · {}", distance_text(row.range, data.draw_distance), band.title()),
        None => distance_text(None, data.draw_distance),
    };
    let color = if row.range.is_some() { band.color(p) } else { p.muted };
    let w = |ui: &egui::Ui, text: &str| -> f32 {
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(12.0), Color32::PLACEHOLDER)
            .size()
            .x
            + 2.0
    };
    line(ui, p, "map-pin-filled", aurora_i18n::tr!("common-distance"), w(ui, &range), |ui| {
        value(ui, range.clone(), color)
    });
    let seen = seen_text(row.seen);
    line(ui, p, "hourglass-filled", aurora_i18n::tr!("radar-col-seen"), w(ui, &seen), |ui| {
        value(ui, seen.clone(), p.muted)
    });
    if row.same_region {
        let yes = aurora_i18n::text("common-yes").to_lowercase();
        line(ui, p, "location-filled", aurora_i18n::tr!("radar-col-region"), w(ui, &yes), |ui| {
            value(ui, yes.clone(), p.muted)
        });
    }
    if let Some(speaking) = row.voice {
        let text = if speaking {
            aurora_i18n::tr!("radar-voice-speaking")
        } else {
            aurora_i18n::tr!("radar-voice-connected")
        };
        let color = if speaking { p.success } else { p.muted };
        line(ui, p, "microphone-filled", aurora_i18n::tr!("common-voice"), w(ui, text), |ui| {
            value(ui, text.to_owned(), color)
        });
    }
    let age = match row.age {
        Age::Unknown => None,
        Age::Hidden => Some((aurora_i18n::tr!("radar-age-hidden").to_owned(), p.muted_dim)),
        Age::Days(days) => {
            // red up to the age alert's threshold, as in the radar table
            let young = (0..=i64::from(data.settings.age_alert_days)).contains(&days);
            Some((
                aurora_i18n::tr!("radar-age-days", days = days),
                if young { p.danger } else { p.muted },
            ))
        }
    };
    if let Some((text, color)) = age {
        line(ui, p, "cake", aurora_i18n::tr!("radar-col-age"), w(ui, &text), |ui| {
            value(ui, text.clone(), color)
        });
    }
    let payment = match row.payment {
        Payment::Unknown => None,
        Payment::None => Some((aurora_i18n::tr!("radar-payment-none"), p.muted_dim)),
        Payment::OnFile => Some((aurora_i18n::tr!("radar-payment-on-file"), p.amber)),
        Payment::Used => Some((aurora_i18n::tr!("radar-payment-used"), p.success)),
    };
    if let Some((text, color)) = payment {
        line(ui, p, "credit-card-filled", aurora_i18n::tr!("radar-col-payment"), 0.0, |_| {});
        // under its label: the three texts are long
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            value(ui, text.to_owned(), color);
        });
        ui.add_space(4.0);
    }
    if !row.note.is_empty() {
        line(ui, p, "note", aurora_i18n::tr!("radar-col-notes"), 0.0, |_| {});
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            ui.add(egui::Label::new(egui::RichText::new(&row.note).size(12.0).color(p.teal)).wrap());
        });
    }
}

/// The map, the payment and the friendship, at the bottom.
fn footer(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid, friend: Option<&Friend>) {
    if id == world.agent_id {
        return;
    }
    ui.spacing_mut().item_spacing.x = 5.0;
    let size = Vec2::new(0.0, BUTTON_H);
    // a friend who lets us see them on the map can be shown there from afar
    let mappable = world.can_locate_avatar(id);
    if widgets::action_button_if(ui, p, "map", aurora_i18n::tr!("common-map"), ActionKind::Flat, size, mappable).clicked() {
        context::request(ui.ctx(), CtxAction::ShowOnMap(id));
    }
    if widgets::action_button(ui, p, "coin-filled", aurora_i18n::tr!("common-pay"), ActionKind::Flat, size).clicked() {
        context::request(ui.ctx(), CtxAction::PayResident(id));
    }
    if friend.is_some() {
        if widgets::action_button(
            ui,
            p,
            "user-minus",
            aurora_i18n::tr!("ui-profile-aurora-remove-friend"),
            ActionKind::SoftDanger,
            size,
        )
        .clicked()
        {
            context::request(ui.ctx(), CtxAction::RemoveFriend(id));
        }
    } else if widgets::action_button(ui, p, "user-plus", aurora_i18n::tr!("radar-add-friend"), ActionKind::Flat, size).clicked() {
        context::request(ui.ctx(), CtxAction::AddFriend(id));
    }
}

/// Draw a sheet with `draw` in a 330 × `height` rectangle for a few frames
/// (the layout settles), then return the rectangle of its vertical scroll
/// bar and the widgets (switches, buttons, values, the cross) it crosses.
#[cfg(test)]
pub(super) fn scroll_bar_crossings(height: f32, mut draw: impl FnMut(&mut egui::Ui)) -> (Option<Rect>, Vec<Rect>) {
    let ctx = egui::Context::default();
    crate::theme::Theme::default().apply(&ctx, 1.0);
    let sheet = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(super::nearby::SHEET_W, height));
    let mut layer = None;
    for _ in 0..4 {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 700.0))),
                // the pointer on the sheet: a floating bar would show over it
                events: vec![egui::Event::PointerMoved(sheet.center())],
                ..Default::default()
            },
            |ui| {
                let mut child = child(ui, sheet, "test-sheet");
                layer = Some(child.layer_id());
                draw(&mut child);
            },
        );
        out.textures_delta.clear();
    }
    let layer = layer.expect("drawn");
    let rects: Vec<egui::WidgetRect> = ctx.viewport(|v| v.this_pass.widgets.get_layer(layer).cloned().collect());
    let bar_w = widgets::reserved_scroll_style().bar_width;
    // the bar: the one dragged widget as wide as the bar
    let bar = rects
        .iter()
        .find(|w| w.sense.senses_drag() && (w.rect.width() - bar_w).abs() < 0.5 && w.rect.height() > 20.0)
        .map(|w| w.rect);
    let Some(bar) = bar else {
        return (None, Vec::new());
    };
    let crossing = rects
        .iter()
        .map(|w| w.rect)
        // the bar itself and the containers around it
        .filter(|r| *r != bar && !r.contains_rect(bar) && r.area() > 0.0)
        .filter(|r| r.intersect(bar).area() > 0.0)
        .collect();
    (Some(bar), crossing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A sheet shorter than its content: its scroll bar is there, in a
    /// strip of its own, crossing no switch, button, value nor the cross,
    /// and the head (picture, names, cross) stays out of what scrolls.
    #[test]
    fn the_scroll_bar_never_covers_a_persons_sheet() {
        let p = crate::theme::Theme::default().palette();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        let friend = Uuid::from_u128(2);
        world.social.friends.push(Friend {
            id: friend,
            online: true,
            rights_given: 3,
            rights_has: 7,
        });
        world.contact_sets.add_set("Family");
        let is_friend = |id: &Uuid| *id == friend;
        world.contact_sets.add_to_set(&[friend], "Family", is_friend);
        let radar = crate::world::radar::Radar::default();
        let mut settings = super::super::RadarSettings::default();
        let (voice, pics) = (HashMap::new(), HashMap::new());
        let data = RadarData {
            radar: &radar,
            settings: &mut settings,
            voice: &voice,
            draw_distance: 64.0,
            now: 0.0,
            pics: &pics,
        };
        let mut st = PeopleUi::default();
        let mut actions = Vec::new();
        for height in [220.0, 300.0, 380.0] {
            let (bar, crossing) = scroll_bar_crossings(height, |ui| {
                show(ui, &p, &world, &data, &mut st, friend, false, true, &mut actions);
            });
            assert!(bar.is_some(), "{height}: the content overflows, the bar shows");
            assert!(crossing.is_empty(), "{height}: the bar {bar:?} crosses {crossing:?}");
        }
        // the head is above what scrolls
        let (top, rest) = split_head(Rect::from_min_size(Pos2::ZERO, Vec2::new(306.0, 300.0)));
        assert!(top.bottom() < rest.top());
    }
}
