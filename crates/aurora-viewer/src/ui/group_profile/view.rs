//! What the window shows of a group, gathered once a frame, and the head of
//! the Aurora look: the banner with the insignia, the name, the founder and
//! the counts, the actions, the badges and the tabs.

use super::{GroupAction, Tab};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::menu;
use crate::ui::place_details::Ctx;
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::group_data::GroupData;
use aurora_net::GroupMembership;
use aurora_net::groups::powers;
use aurora_net::search::GroupProfile;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, FontId, Pos2, Rect, Sense, Shape, Vec2};
use uuid::Uuid;

/// Height of the banner, the insignia's side and where it starts below the
/// banner's top; the identity starts this far from the left edge.
const BANNER_H: f32 = 132.0;
const INSIGNIA: f32 = 116.0;
const INSIGNIA_TOP: f32 = 66.0;
const INSIGNIA_LEFT: f32 = 12.0;
const BORDER: f32 = 4.0;
const IDENTITY_LEFT: f32 = 150.0;
const BUTTON_H: f32 = 28.0;

pub struct View<'a> {
    pub id: Uuid,
    pub name: String,
    pub profile: Option<&'a GroupProfile>,
    /// Our membership (AgentGroupDataUpdate): None for a guest.
    pub member: Option<&'a GroupMembership>,
    pub data: Option<&'a GroupData>,
    pub active: bool,
}

impl<'a> View<'a> {
    pub fn of(world: &'a World, id: Uuid) -> View<'a> {
        let profile = world.search.groups.get(&id);
        let member = world.groups.group(&id);
        // until the profile answers: the name of one of our groups, or the
        // one "About Land" looked up
        let name = profile
            .map(|g| g.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| member.map(|g| g.name.clone()))
            .or_else(|| world.land.group_names.get(&id).cloned())
            .unwrap_or_default();
        View {
            id,
            name,
            profile,
            member,
            data: world.group_data.get(&id),
            active: world.groups.active == id && !id.is_nil(),
        }
    }

    /// Our powers in the group (LLAgent::hasPowerInGroup).
    pub fn powers(&self) -> u64 {
        self.member.map_or(0, |m| m.powers)
    }

    pub fn can(&self, power: u64) -> bool {
        self.powers() & power != 0
    }

    /// The members online, once the list is complete.
    pub fn online_count(&self) -> Option<usize> {
        self.data.filter(|d| d.members_complete).map(|d| d.online().count())
    }

    /// The member count: the list's once complete, else the profile's.
    pub fn member_count(&self) -> Option<i64> {
        match (self.data.filter(|d| d.members_complete), self.profile) {
            (Some(d), _) => Some(d.member_total.max(0) as i64),
            (None, Some(p)) => Some(i64::from(p.members.max(0))),
            _ => None,
        }
    }

    /// The title we wear: the selected one of GroupTitlesReply, else the
    /// profile's.
    pub fn my_title(&self) -> Option<String> {
        self.data
            .and_then(|d| d.selected_title())
            .map(|t| t.title.clone())
            .or_else(|| self.profile.map(|p| p.member_title.clone()))
            .filter(|t| !t.is_empty())
    }
}

/// The group's rating, in the words of the maturity badges.
pub fn rating(profile: Option<&GroupProfile>) -> &'static str {
    if profile.is_some_and(|g| g.mature_publish) {
        aurora_i18n::tr!("common-moderate")
    } else {
        aurora_i18n::tr!("common-general")
    }
}

fn vertex(pos: Pos2, color: Color32) -> Vertex {
    Vertex { pos, uv: WHITE_UV, color }
}

/// A soft glow: `color` at the centre of an ellipse, transparent at its edge.
fn glow(center: Pos2, radius: Vec2, color: Color32) -> Shape {
    const SEGMENTS: u32 = 40;
    let mut m = Mesh::default();
    m.vertices.push(vertex(center, color));
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        m.vertices.push(vertex(
            center + Vec2::new(a.cos() * radius.x, a.sin() * radius.y),
            Color32::TRANSPARENT,
        ));
        if i > 0 {
            m.add_triangle(0, i, i + 1);
        }
    }
    Shape::mesh(m)
}

/// A vertical gradient over `r`, `top` to `bottom`.
fn vertical(r: Rect, top: Color32, bottom: Color32) -> Shape {
    let mut m = Mesh::default();
    m.vertices.extend([
        vertex(r.left_top(), top),
        vertex(r.right_top(), top),
        vertex(r.right_bottom(), bottom),
        vertex(r.left_bottom(), bottom),
    ]);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    Shape::mesh(m)
}

/// The banner: the insignia blurred across it (a box blur of shifted
/// copies, each drawn over the previous ones at 1/k so that the k copies
/// average), or the aurora's glows while there is none, then darkening
/// toward the identity below.
fn banner(ui: &egui::Ui, p: &Palette, rect: Rect, insignia: Option<&egui::TextureHandle>) {
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter_at(rect);
    match insignia {
        Some(t) => {
            painter.rect_filled(rect, radius, p.panel);
            // the picture covers the banner and 30 px around it (the blur's
            // reach), square
            let side = rect.width() + 60.0;
            let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
            let uv = |r: Rect| Rect::from_min_max(((r.min - square.min) / side).to_pos2(), ((r.max - square.min) / side).to_pos2());
            const STEPS: i32 = 5;
            const REACH: f32 = 26.0;
            let mut k = 0.0;
            for y in 0..STEPS {
                for x in 0..STEPS {
                    let d = Vec2::new(x as f32, y as f32) / (STEPS - 1) as f32 * 2.0 - Vec2::splat(1.0);
                    k += 1.0;
                    egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                        .uv(uv(rect.translate(d * REACH)))
                        .tint(widgets::PICTURE_TINT.gamma_multiply(0.9 / k))
                        .corner_radius(radius)
                        .paint_at(ui, rect);
                }
            }
        }
        None => {
            painter.rect_filled(rect, radius, p.navy.lerp_to_gamma(p.indigo, 0.25));
            let h = rect.height();
            painter.add(glow(
                rect.left_top() + Vec2::new(rect.width() * 0.25, h * 0.6),
                Vec2::new(rect.width() * 0.42, h * 1.8),
                p.violet_light.gamma_multiply(0.95),
            ));
            painter.add(glow(
                rect.left_top() + Vec2::new(rect.width() * 0.75, h * 0.3),
                Vec2::new(rect.width() * 0.42, h * 1.8),
                p.teal.gamma_multiply(0.85),
            ));
        }
    }
    let fade = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.3), rect.right_bottom());
    painter.add(vertical(fade, Color32::TRANSPARENT, p.panel.gamma_multiply(0.85)));
}

/// The insignia over the banner's edge, framed in the window's color, or
/// the group's initials on the accent while it has none.
fn insignia(ui: &egui::Ui, p: &Palette, rect: Rect, picture: Option<&egui::TextureHandle>, name: &str) {
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, p.panel.lerp_to_gamma(p.raised, 0.12));
    let inner = rect.shrink(BORDER);
    match picture {
        Some(t) => {
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), inner.size()))
                .tint(widgets::PICTURE_TINT)
                .corner_radius(radius)
                .paint_at(ui, inner);
        }
        None => {
            // the accent from the top left to the bottom right
            let mut m = Mesh::default();
            let (a, b) = (p.violet_light, p.indigo);
            let mid = a.lerp_to_gamma(b, 0.5);
            m.vertices.extend([
                vertex(inner.left_top(), a),
                vertex(inner.right_top(), mid),
                vertex(inner.right_bottom(), b),
                vertex(inner.left_bottom(), mid),
            ]);
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            painter.add(Shape::mesh(m));
            let font = FontId::new(34.0, crate::ui::fonts::semibold(ui.ctx()));
            let galley = painter.layout_no_wrap(widgets::initials(name), font, p.on_accent);
            widgets::paint_ink_centered(painter, galley, inner.center(), p.on_accent);
        }
    }
}

/// A badge of the header: an icon (or a mark painted by `lead`) and a text,
/// on a pill.
pub fn badge(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str, ink: Color32, fill: Color32) -> egui::Response {
    badge_with(ui, p, text, ink, fill, 12.0, |ui, r| {
        if let Some(t) = crate::ui::icons::global(icon) {
            ui.painter()
                .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), ink);
        }
    })
}

fn badge_with(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &str,
    ink: Color32,
    fill: Color32,
    lead: f32,
    paint_lead: impl FnOnce(&egui::Ui, Rect),
) -> egui::Response {
    let font = FontId::new(11.5, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let gap = if lead > 0.0 { 4.0 } else { 0.0 };
    let size = Vec2::new(8.0 + lead + gap + galley.size().x + 8.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let _ = p;
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    if lead > 0.0 {
        paint_lead(
            ui,
            Rect::from_center_size(Pos2::new(rect.left() + 8.0 + lead / 2.0, rect.center().y), Vec2::splat(lead)),
        );
    }
    let pos = Pos2::new(rect.left() + 8.0 + lead + gap, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos.round(), galley, ink);
    resp
}

/// The head of the window. Returns the group to join when "Join" was
/// clicked (the window confirms it first).
pub fn header(ui: &mut egui::Ui, c: &mut Ctx, v: &View, env: &super::Env, tab: &mut Tab, actions: &mut Vec<GroupAction>) -> Option<Header> {
    let p = c.p;
    let mut clicked = None;
    let top = ui.cursor().min;
    let width = ui.available_width();
    let banner_rect = Rect::from_min_size(top, Vec2::new(width, BANNER_H));
    let pic = v.profile.map(|g| g.insignia).filter(|i| !i.is_nil());
    if let Some(i) = pic {
        c.wanted_images.insert(i);
    }
    let tex = pic.and_then(|i| c.images.get(&i));
    banner(ui, p, banner_rect, tex);
    let insignia_rect = Rect::from_min_size(top + Vec2::new(INSIGNIA_LEFT, INSIGNIA_TOP), Vec2::splat(INSIGNIA));

    // the identity: name, founder and counts on the left, the actions on
    // the right
    let id_top = banner_rect.bottom() + 10.0;
    let id_left = top.x + IDENTITY_LEFT;
    let row = Rect::from_min_max(Pos2::new(id_left, id_top), Pos2::new(banner_rect.right() - 4.0, id_top + 56.0));
    let mut acts = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::right_to_left(egui::Align::Min)),
    );
    acts.spacing_mut().item_spacing.x = 5.0;
    let size = Vec2::new(0.0, BUTTON_H);
    let more = widgets::action_button(
        &mut acts,
        p,
        "dots-filled",
        "",
        ActionKind::Flat,
        Vec2::new(BUTTON_H + 4.0, BUTTON_H),
    )
    .on_hover_text(aurora_i18n::tr!("ui-profile-more"));
    more_menu(&more, c, v, actions);
    if v.member.is_some() {
        // LLGroupActions::leave, confirmed by the shared dialog
        if widgets::action_button(
            &mut acts,
            p,
            "door-exit",
            aurora_i18n::tr!("ui-group-profile-leave"),
            ActionKind::SoftDanger,
            size,
        )
        .clicked()
        {
            context::request(acts.ctx(), CtxAction::LeaveGroup(v.id));
        }
        if v.can(powers::MEMBER_INVITE)
            && widgets::action_button(
                &mut acts,
                p,
                "user-plus",
                aurora_i18n::tr!("ui-group-profile-invite"),
                ActionKind::Flat,
                size,
            )
            .clicked()
        {
            clicked = Some(Header::Invite);
        }
        // LLGroupActions::startCall: group voice comes with the voice calls
        // (`Env::group_calls`)
        let call = widgets::action_button_if(
            &mut acts,
            p,
            "phone-filled",
            aurora_i18n::tr!("ui-group-profile-call"),
            ActionKind::Flat,
            size,
            env.group_calls,
        )
        .on_disabled_hover_text(menu::not_yet());
        if call.clicked() {
            actions.push(GroupAction::Call(v.id));
        }
        if widgets::action_button(
            &mut acts,
            p,
            "messages-filled",
            aurora_i18n::tr!("common-chat"),
            ActionKind::Primary,
            size,
        )
        .clicked()
        {
            context::request(acts.ctx(), CtxAction::GroupChat(v.id));
        }
    } else if let Some(g) = v.profile.filter(|g| g.open_enrollment) {
        // LLPanelGroup::update: "Join" only for an open enrollment, its fee
        // in the label
        let label = if g.membership_fee > 0 {
            aurora_i18n::tr!("ui-group-profile-join-fee", fee = aurora_i18n::integer(i64::from(g.membership_fee)))
        } else {
            aurora_i18n::tr!("ui-group-profile-join").to_owned()
        };
        if widgets::action_button(&mut acts, p, "door-enter", &label, ActionKind::Primary, size).clicked() {
            clicked = Some(Header::Join);
        }
    }
    let buttons_left = acts.min_rect().left();
    let who = Rect::from_min_max(
        Pos2::new(id_left, id_top),
        Pos2::new((buttons_left - 14.0).max(id_left + 40.0), id_top + 56.0),
    );
    let mut who_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(who)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    who_ui.spacing_mut().item_spacing.y = 1.0;
    let name_font = FontId::new(22.0, crate::ui::fonts::semibold(ui.ctx()));
    let shown = if v.name.is_empty() {
        aurora_i18n::tr!("common-loading-2").to_owned()
    } else {
        v.name.clone()
    };
    who_ui.add(egui::Label::new(egui::RichText::new(&shown).font(name_font).color(p.ink)).truncate());
    who_ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let small = |t: String| egui::RichText::new(t).size(12.5).color(p.muted);
        if let Some(g) = v.profile.filter(|g| !g.founder.is_nil()) {
            ui.label(small(aurora_i18n::tr!("ui-group-profile-founded-by").to_owned() + " "));
            c.want_names.insert(g.founder);
            let founder = c.world.social.display_name_of(&g.founder).to_string();
            let link = ui.add(egui::Label::new(egui::RichText::new(founder).size(12.5).color(p.violet_light)).sense(Sense::click()));
            if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                crate::ui::profile::request_open(ui.ctx(), g.founder);
            }
        }
        if let Some(n) = v.member_count() {
            if v.profile.is_some_and(|g| !g.founder.is_nil()) {
                ui.label(small(" · ".into()));
            }
            ui.label(small(aurora_i18n::tr!(
                "ui-group-profile-members",
                count = n,
                shown = aurora_i18n::integer(n)
            )));
        }
        if let Some(online) = v.online_count() {
            ui.label(small(" · ".into()));
            ui.label(
                egui::RichText::new(aurora_i18n::tr!(
                    "ui-group-profile-online",
                    shown = aurora_i18n::integer(online as i64)
                ))
                .size(12.5)
                .color(p.success),
            );
        }
    });
    // the insignia goes over the identity's column edge, after the banner
    insignia(ui, p, insignia_rect, tex, &v.name);

    // the badges
    let badges_top = id_top + 56.0 + 4.0;
    let badges = Rect::from_min_max(
        Pos2::new(id_left, badges_top),
        Pos2::new(banner_rect.right() - 4.0, badges_top + 40.0),
    );
    let mut b = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(badges)
            .layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true)),
    );
    b.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
    b.add_space(0.0);
    badges_row(&mut b, c, v, actions);
    let used = b.min_rect().bottom().max(badges_top + 40.0);
    ui.allocate_rect(Rect::from_min_max(top, Pos2::new(banner_rect.right(), used)), Sense::hover());
    tabs(ui, p, v, tab, c.world.group_data.unread(&v.id));
    clicked
}

/// What the head's buttons ask of the window itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Header {
    /// "Join", confirmed first.
    Join,
    /// "Invite": the invitation window.
    Invite,
}

/// Content, enrollment, fee; our membership and title; the active group;
/// "See in People".
fn badges_row(ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let neutral = (p.muted, p.bar);
    let rating = rating(v.profile);
    let content = if v.profile.is_some_and(|g| g.mature_publish) {
        aurora_i18n::tr!("ui-group-profile-moderate-content")
    } else {
        aurora_i18n::tr!("ui-group-profile-general-content")
    };
    badge_with(ui, p, content, neutral.0, neutral.1, 13.0, |ui, r| {
        widgets::paint_maturity_badge(ui.painter(), p, rating, r, widgets::inner_radius(ui.visuals()));
    });
    if let Some(g) = v.profile {
        if g.open_enrollment {
            badge(
                ui,
                p,
                "door-enter",
                aurora_i18n::tr!("ui-group-profile-open-enrollment"),
                neutral.0,
                neutral.1,
            );
        } else {
            badge(
                ui,
                p,
                "lock-filled",
                aurora_i18n::tr!("ui-group-profile-invitation-only"),
                neutral.0,
                neutral.1,
            );
        }
        let fee = if g.membership_fee > 0 {
            aurora_i18n::tr!("ui-group-profile-fee", fee = aurora_i18n::integer(i64::from(g.membership_fee)))
        } else {
            aurora_i18n::tr!("ui-group-profile-free").to_owned()
        };
        badge(ui, p, "coin-filled", &fee, neutral.0, neutral.1);
    }
    if v.member.is_some() {
        let text = match v.my_title() {
            Some(t) => aurora_i18n::tr!("ui-group-profile-member-as", title = t),
            None => aurora_i18n::tr!("ui-group-profile-member").to_owned(),
        };
        badge(ui, p, "check", &text, p.success, p.success.gamma_multiply(0.12));
    }
    if v.active {
        badge(
            ui,
            p,
            "star-filled",
            aurora_i18n::tr!("ui-group-profile-active"),
            p.teal,
            p.teal.gamma_multiply(0.14),
        );
    }
    if v.member.is_some() {
        let link = see_in_people(ui, p);
        if link.clicked() {
            actions.push(GroupAction::ShowInPeople(v.name.clone()));
        }
    }
}

/// "See in People ›": a link in the accent.
fn see_in_people(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    let font = FontId::new(11.5, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui
        .painter()
        .layout_no_wrap(aurora_i18n::tr!("ui-group-profile-see-in-people").to_owned(), font, p.violet_light);
    let size = Vec2::new(2.0 + galley.size().x + 4.0 + 12.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let color = if resp.hovered() { p.violet_pale } else { p.violet_light };
    let w = galley.size().x;
    ui.painter().galley(
        Pos2::new(rect.left() + 2.0, rect.center().y - galley.size().y / 2.0).round(),
        galley,
        color,
    );
    if let Some(t) = crate::ui::icons::global("chevron-right") {
        let r = Rect::from_center_size(Pos2::new(rect.left() + 2.0 + w + 4.0 + 6.0, rect.center().y), Vec2::splat(12.0));
        ui.painter()
            .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "…": copy the URI, the name, the key; refresh; see in People.
fn more_menu(resp: &egui::Response, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    menu::dropdown(resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("search-group-copy-uri")) {
            ui.ctx().copy_text(format!("secondlife:///app/group/{}/about", v.id));
        }
        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("search-group-copy-name")) {
            ui.ctx().copy_text(v.name.clone());
        }
        if menu::item(ui, p, "key-filled", aurora_i18n::tr!("search-copy-key")) {
            ui.ctx().copy_text(v.id.to_string());
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "refresh", aurora_i18n::tr!("search-refresh")) {
            actions.push(GroupAction::Refresh {
                group: v.id,
                member: v.member.is_some(),
            });
        }
        if menu::item_if(
            ui,
            p,
            "users-group",
            aurora_i18n::tr!("ui-group-profile-see-in-people"),
            v.member.is_some(),
        ) {
            actions.push(GroupAction::ShowInPeople(v.name.clone()));
        }
    });
}

/// One tab of the row: its icon, its name, a count after it.
fn tab_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    count: Option<String>,
    accent: bool,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let ink = if !enabled {
        p.muted_dim
    } else if selected {
        p.ink
    } else {
        p.muted
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    // an unread count is a pill in the accent (`accent`)
    let count_ink = if accent { p.on_accent } else { p.muted_dim };
    let count = count.map(|t| {
        ui.painter()
            .layout_no_wrap(t, FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx())), count_ink)
    });
    let pad = if accent { 5.0 } else { 0.0 };
    let count_w = count.as_ref().map_or(0.0, |g| 5.0 + g.size().x + pad * 2.0);
    let size = Vec2::new(12.0 + 15.0 + 5.0 + galley.size().x + count_w + 12.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let mut x = rect.left() + 12.0;
    if let Some(t) = crate::ui::icons::global(icon) {
        let r = Rect::from_center_size(Pos2::new(x + 7.5, rect.center().y), Vec2::splat(15.0));
        ui.painter()
            .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), ink);
    }
    x += 15.0 + 5.0;
    let w = galley.size().x;
    ui.painter()
        .galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0).round(), galley, ink);
    if let Some(g) = count {
        let pos = Pos2::new(x + w + 5.0 + pad, rect.center().y - g.size().y / 2.0);
        if accent {
            let pill = Rect::from_min_size(
                Pos2::new(x + w + 5.0, rect.center().y - 8.0),
                Vec2::new(g.size().x + pad * 2.0, 16.0),
            );
            ui.painter().rect_filled(pill, ui.visuals().window_corner_radius, p.violet);
        }
        ui.painter().galley(pos.round(), g, count_ink);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, label));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp.on_hover_text(menu::not_yet())
    }
}

/// Home, Members and the tabs still to port (greyed: notices, land and
/// L$, experiences).
fn tabs(ui: &mut egui::Ui, p: &Palette, v: &View, tab: &mut Tab, unread: usize) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        if tab_button(
            ui,
            p,
            "home-filled",
            aurora_i18n::tr!("ui-group-profile-home"),
            None,
            false,
            *tab == Tab::Home,
            true,
        )
        .clicked()
        {
            *tab = Tab::Home;
        }
        let count = v.member_count().map(aurora_i18n::integer);
        if tab_button(
            ui,
            p,
            "users",
            aurora_i18n::tr!("ui-group-profile-members-tab"),
            count,
            false,
            *tab == Tab::Members,
            true,
        )
        .clicked()
        {
            *tab = Tab::Members;
        }
        // the unread notices on the tab
        let unread = (unread > 0).then(|| aurora_i18n::integer(unread as i64));
        if tab_button(
            ui,
            p,
            "speakerphone",
            aurora_i18n::tr!("ui-group-profile-notices"),
            unread,
            true,
            *tab == Tab::Notices,
            super::notices::visible(v),
        )
        .clicked()
        {
            *tab = Tab::Notices;
        }
        tab_button(ui, p, "map-2", aurora_i18n::tr!("ui-group-profile-land"), None, false, false, false);
        tab_button(
            ui,
            p,
            "flask-filled",
            aurora_i18n::tr!("ui-group-profile-experiences"),
            None,
            false,
            false,
            false,
        );
    });
    ui.add_space(8.0);
}

/// A card of the tabs: its title in small capitals with an icon, a link at
/// the right of the title, then the body.
pub fn card<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    icon_color: Color32,
    title: &str,
    link: Option<&str>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let mut clicked = false;
    let inner = egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                widgets::glyph(ui, icon, icon_color, 13.0);
                let mut job = egui::text::LayoutJob::default();
                let mut format = crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, p.muted_dim);
                format.extra_letter_spacing = 0.4;
                job.append(&title.to_uppercase(), 0.0, format);
                ui.label(job);
                if let Some(link) = link {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.add(egui::Label::new(egui::RichText::new(link).size(11.5).color(p.violet_light)).sense(Sense::click()));
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            clicked = true;
                        }
                    });
                }
            });
            ui.add_space(2.0);
            body(ui)
        });
    (inner.inner, clicked)
}

/// A role chip: the owners' in amber, the officers' in violet, the others
/// neutral.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
    Owner,
    Officer,
    Plain,
}

pub fn role_chip(ui: &mut egui::Ui, p: &Palette, text: &str, chip: Chip) -> egui::Response {
    let (ink, fill) = match chip {
        Chip::Owner => (p.amber, p.amber.gamma_multiply(0.14)),
        Chip::Officer => (p.violet_pale, p.violet.gamma_multiply(0.2)),
        Chip::Plain => (p.muted, p.raised),
    };
    let font = FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 12.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    let pos = Pos2::new(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos.round(), galley, ink);
    resp
}
