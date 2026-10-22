//! "Create a group" (LLPanelGroupCreate, indra/newview/llpanelgroupcreate.cpp
//! and panel_group_creation_sidetray.xml; LLGroupActions::createGroup,
//! LLGroupMgr::sendCreateGroupRequest / processCreateGroupReply,
//! originally LGPL 2.1).
//!
//! Firestorm's rules: a name of 4 to 35 ASCII characters, not starting with
//! a space (GroupNameLengthWarning once trimmed); a content rating to
//! choose, asked when left unchosen (SetGroupMature: Yes / No / Cancel); the
//! creation fee of the account's membership (`create_group_cost` of the
//! login's benefits, 0 when the login gives none), the fees of the other
//! memberships listed when the login gives them (`premium_packages`); the
//! group limit (JoinedTooManyGroups); the server's refusal shown with its
//! reason (UnableToCreateGroup); on success the new group's profile opened
//! and the balance asked again (refreshCreatedGroup).
//!
//! Deliberate differences: the fee is confirmed before the request leaves
//! (Firestorm's CreateGroupCost notification, which its code no longer
//! raises: its button carries the fee); the request is sent once, the
//! button greyed until the reply (Firestorm sends it again at each click);
//! the Aurora look adds "Show in search" (Firestorm always sends it on).
//!
//! Two looks, as the profile: the Aurora one is the profile's header (the
//! banner, the insignia, the name being typed, the badges) over the cards of
//! Administration › Group identity and a fee card; the classic one is
//! Firestorm's creation panel.

use super::administration::{self, IdentityFields, Rating};
use super::view;
use super::{Env, GroupAction};
use crate::theme::Palette;
use crate::ui::place_details::Ctx;
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::{self, ActionKind, DialogButton, Floater};
use crate::world::World;
use aurora_llsd::Llsd;
use aurora_net::groups::{GroupInfo, name};
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

pub const WINDOW: &str = "group_create";

/// The fees and the charter limit of the creation panel.
const CHARTER_BYTES: usize = 511;
const FEE_MAX: i32 = 99_999;

/// Where the membership page is ("fee_information"'s link).
const MEMBERSHIP_URL: &str = "https://secondlife.com/my/account/membership.php";

/// The creation fee of a membership level (LLPanelGroupCreate::addMembershipRow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// The package: "Base", "Premium", "Premium_Plus", "Internal".
    pub package: String,
    pub cost: i32,
    pub current: bool,
}

/// LLAgentBenefits::getCreateGroupCost: the account's fee, 0 when the
/// login gives none.
pub fn fee_of(raw: &Llsd) -> i32 {
    raw["account_level_benefits"]["create_group_cost"].as_i32().max(0)
}

/// The fees of the memberships the login knows, in Firestorm's order
/// (Base, Premium, Premium Plus, Internal), ours marked; the account's own
/// level when the packages are not given.
pub fn levels_of(raw: &Llsd) -> Vec<Level> {
    let current = raw["account_type"].as_str();
    let packages = &raw["premium_packages"];
    let mut levels: Vec<Level> = ["Base", "Premium", "Premium_Plus", "Internal"]
        .into_iter()
        .filter(|name| packages.has(name))
        .map(|name| Level {
            package: name.to_owned(),
            cost: packages[name]["benefits"]["create_group_cost"].as_i32().max(0),
            current: name == current,
        })
        .collect();
    if levels.is_empty() && raw["account_level_benefits"].has("create_group_cost") && !current.is_empty() {
        levels.push(Level {
            package: current.to_owned(),
            cost: fee_of(raw),
            current: true,
        });
    }
    levels
}

/// The account's fee.
pub fn fee(world: &World) -> i32 {
    world.login.as_ref().map_or(0, |l| fee_of(&l.raw))
}

/// LLAgent::canJoinGroups: under the group limit of the account.
pub fn can_join_groups(world: &World) -> bool {
    let limit = crate::ui::contacts::group_limit(world);
    limit == 0 || world.groups.groups.len() < limit
}

/// A membership's name (strings.xml BaseMembership…).
fn level_name(package: &str) -> String {
    match package {
        "Base" => aurora_i18n::tr!("ui-group-create-level-base").to_owned(),
        "Premium" => aurora_i18n::tr!("ui-group-create-level-premium").to_owned(),
        "Premium_Plus" => aurora_i18n::tr!("ui-group-create-level-premium-plus").to_owned(),
        other => other.replace('_', " "),
    }
}

/// The group being written.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub name: String,
    /// Charter, insignia, search, enrollment and fee, as UpdateGroupInfo.
    pub info: GroupInfo,
    /// The content rating chosen (true: Moderate); None until then.
    pub rating: Option<bool>,
}

impl Default for Draft {
    /// LLPanelGroupCreate::onOpen: everything empty, no fee, invitation
    /// only, the rating to choose; shown in search, as Firestorm sends it.
    fn default() -> Self {
        Draft {
            name: String::new(),
            info: GroupInfo {
                show_in_list: true,
                ..Default::default()
            },
            rating: None,
        }
    }
}

/// A dialog of the creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// GroupNameLengthWarning.
    NameLength,
    /// SetGroupMature: Yes, No or Cancel.
    Mature,
    /// CreateGroupCost.
    Cost(i32),
    /// UnableToCreateGroup, with the server's message.
    Refused(String),
    /// JoinedTooManyGroups.
    TooManyGroups,
}

/// What a click on "Create" leads to (onBtnCreate, then the fee).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Ask(Dialog),
    Send,
}

/// onBtnCreate: the name's length, the group limit, the rating, the fee.
pub fn next(draft: &Draft, can_join: bool, fee: i32) -> Next {
    if name::checked(&draft.name).is_none() {
        Next::Ask(Dialog::NameLength)
    } else if !can_join {
        Next::Ask(Dialog::TooManyGroups)
    } else if draft.rating.is_none() {
        Next::Ask(Dialog::Mature)
    } else if fee > 0 {
        Next::Ask(Dialog::Cost(fee))
    } else {
        Next::Send
    }
}

/// The longest start of `text` within `max` bytes.
fn clip_bytes(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

#[derive(Default)]
pub struct GroupCreateUi {
    open: bool,
    raise: bool,
    pub draft: Draft,
    dialog: Option<Dialog>,
    /// The request left: Create stays greyed until the reply.
    sending: bool,
    texture: TexturePicker,
    /// The demo scenario, started once the group list is in.
    demo: Option<DemoStart>,
    /// The last edit of the name was refused (an accent, a leading space,
    /// too long): the rule is shown.
    refused_key: bool,
}

/// How a demo scenario starts.
pub struct DemoStart {
    pub draft: Option<Draft>,
    pub then: DemoThen,
}

/// What the demo does with the form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DemoThen {
    Wait,
    /// "Create" clicked (the fee to confirm).
    Press,
    /// The request sent, as if confirmed.
    Send,
}

impl GroupCreateUi {
    /// LLGroupActions::createGroup: an empty form (onOpen), or
    /// JoinedTooManyGroups at the group limit (People's "+" validation).
    pub fn open(&mut self, world: &World) {
        if !can_join_groups(world) {
            self.dialog = Some(Dialog::TooManyGroups);
            return;
        }
        // an open window is only brought to the front, its draft kept
        if !self.open {
            self.draft = Draft::default();
            self.sending = false;
            self.refused_key = false;
            self.dialog = None;
        }
        self.open = true;
        self.raise = true;
    }

    /// The demo: the window opened as from People once the group list is
    /// in, then filled, "Create" clicked or the request sent.
    pub fn demo(&mut self, start: DemoStart) {
        self.demo = Some(start);
    }

    fn start_demo(&mut self, world: &World, actions: &mut Vec<GroupAction>) {
        if world.groups.groups.is_empty() {
            return;
        }
        let Some(start) = self.demo.take() else {
            return;
        };
        self.open(world);
        if !self.open {
            return;
        }
        if let Some(draft) = start.draft {
            self.draft = draft;
        }
        match start.then {
            DemoThen::Wait => {}
            DemoThen::Press => self.press(world, actions),
            DemoThen::Send => self.send(actions),
        }
    }

    /// processCreateGroupReply: the group to open, or the refusal shown with
    /// the form kept.
    pub fn on_reply(&mut self, group: Uuid, success: bool, message: &str) -> Option<Uuid> {
        self.sending = false;
        if success {
            self.open = false;
            self.dialog = None;
            self.draft = Draft::default();
            Some(group).filter(|g| !g.is_nil())
        } else {
            self.dialog = Some(Dialog::Refused(message.to_owned()));
            None
        }
    }

    /// The insignia chosen in the texture picker (after the windows).
    pub fn picker(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> std::collections::HashSet<Uuid> {
        if let Some(texture) = self.texture.show(ctx, p, world, images) {
            self.draft.info.insignia = texture;
        }
        std::mem::take(&mut self.texture.wanted_images)
    }

    fn pick_insignia(&mut self) {
        self.texture
            .open(aurora_i18n::tr!("ui-group-admin-insignia"), self.draft.info.insignia);
    }

    /// A click on "Create".
    fn press(&mut self, world: &World, actions: &mut Vec<GroupAction>) {
        match next(&self.draft, can_join_groups(world), fee(world)) {
            Next::Ask(d) => self.dialog = Some(d),
            Next::Send => self.send(actions),
        }
    }

    /// LLPanelGroupCreate::createGroup: the trimmed name, the fee only when
    /// asked for.
    fn send(&mut self, actions: &mut Vec<GroupAction>) {
        let Some(name) = name::checked(&self.draft.name) else {
            self.dialog = Some(Dialog::NameLength);
            return;
        };
        let mut info = self.draft.info.clone();
        info.mature_publish = self.draft.rating == Some(true);
        info.allow_publish = false;
        info.membership_fee = info.membership_fee.clamp(0, FEE_MAX);
        actions.push(GroupAction::Create {
            name: name.to_owned(),
            info,
        });
        self.sending = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, c: &mut Ctx, env: &Env) -> Vec<GroupAction> {
        let mut actions = Vec::new();
        if self.demo.is_some() {
            self.start_demo(c.world, &mut actions);
        }
        if self.open {
            if std::mem::take(&mut self.raise) {
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW)));
            }
            let screen = ctx.content_rect();
            let mut open = self.open;
            if env.classic {
                let size = Vec2::new(330.0, 640.0);
                let mut floater = Floater::new(
                    WINDOW,
                    aurora_i18n::tr!("ui-group-create-classic-title"),
                    screen.center() - size * 0.5 + Vec2::new(40.0, 10.0),
                    size,
                );
                floater.min_size = Vec2::new(300.0, 420.0);
                floater.show(ctx, c.p, &mut open, |ui| self.classic(ui, c, &mut actions));
            } else {
                let size = Vec2::new(820.0, 720.0);
                let mut floater = Floater::new(
                    WINDOW,
                    aurora_i18n::tr!("ui-group-create-title"),
                    screen.center() - size * 0.5,
                    size,
                );
                floater.min_size = Vec2::new(560.0, 460.0);
                floater.show(ctx, c.p, &mut open, |ui| self.aurora(ui, c, &mut actions));
            }
            self.open = open;
        }
        self.dialog(ctx, c.p, c.world, &mut actions);
        actions
    }

    /// The name field: a refused edit leaves the field as it was.
    fn name_edit(&mut self, ui: &mut egui::Ui, font: Option<FontId>, width: f32) -> egui::Response {
        let before = self.draft.name.clone();
        let mut edit = egui::TextEdit::singleline(&mut self.draft.name)
            .hint_text(aurora_i18n::tr!("ui-group-create-name-hint"))
            .desired_width(width)
            .interactive(!self.sending);
        if let Some(font) = font {
            edit = edit.font(font);
        }
        let resp = ui.add(edit);
        if resp.changed() {
            self.refused_key = !name::accepts(&self.draft.name);
            if self.refused_key {
                self.draft.name = before;
            }
        }
        resp
    }

    /// The label of "Create": its fee when there is one.
    fn create_label(world: &World) -> String {
        match fee(world) {
            0 => aurora_i18n::tr!("ui-group-create-create").to_owned(),
            n => aurora_i18n::tr!("ui-group-create-create-for", cost = aurora_i18n::integer(i64::from(n))),
        }
    }

    // ------------------------------------------------------------ Aurora look

    fn aurora(&mut self, ui: &mut egui::Ui, c: &mut Ctx, actions: &mut Vec<GroupAction>) {
        let p = c.p;
        let top = ui.cursor().min;
        let width = ui.available_width();
        let banner_rect = Rect::from_min_size(top, Vec2::new(width, view::BANNER_H));
        let insignia = self.draft.info.insignia;
        if !insignia.is_nil() {
            c.wanted_images.insert(insignia);
        }
        let tex = (!insignia.is_nil()).then(|| c.images.get(&insignia)).flatten();
        view::banner(ui, p, banner_rect, tex);
        let insignia_rect = Rect::from_min_size(
            top + Vec2::new(view::INSIGNIA_LEFT, view::INSIGNIA_TOP),
            Vec2::splat(view::INSIGNIA),
        );

        // the name being typed on the left, Create and Cancel on the right
        let id_top = banner_rect.bottom() + 10.0;
        let id_left = top.x + view::IDENTITY_LEFT;
        let row = Rect::from_min_max(Pos2::new(id_left, id_top), Pos2::new(banner_rect.right() - 4.0, id_top + 56.0));
        let mut acts = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::right_to_left(egui::Align::Min)),
        );
        acts.spacing_mut().item_spacing.x = 5.0;
        let size = Vec2::new(0.0, view::BUTTON_H);
        if widgets::action_button(&mut acts, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
            self.open = false;
        }
        let label = if self.sending {
            aurora_i18n::tr!("ui-group-create-creating").to_owned()
        } else {
            Self::create_label(c.world)
        };
        if widgets::action_button_if(&mut acts, p, "plus", &label, ActionKind::Primary, size, !self.sending).clicked() {
            self.press(c.world, actions);
        }
        let buttons_left = acts.min_rect().left();
        let who = Rect::from_min_max(
            Pos2::new(id_left, id_top),
            Pos2::new((buttons_left - 14.0).max(id_left + 120.0), id_top + 56.0),
        );
        let mut who_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(who)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        who_ui.spacing_mut().item_spacing.y = 2.0;
        let name_font = FontId::new(20.0, crate::ui::fonts::semibold(ui.ctx()));
        let w = who_ui.available_width();
        self.name_edit(&mut who_ui, Some(name_font), w);
        who_ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let small = |t: String| egui::RichText::new(t).size(12.0).color(p.muted);
            // the founder and the count, or the rule in amber when a key was
            // refused or the name is too short
            let len = self.draft.name.trim().len();
            let rule = self.refused_key || (len > 0 && name::checked(&self.draft.name).is_none());
            let me = c.world.agent_id;
            if !me.is_nil() && !rule {
                c.want_names.insert(me);
                ui.label(small(aurora_i18n::tr!("ui-group-profile-founded-by").to_owned() + " "));
                ui.label(
                    egui::RichText::new(c.world.social.display_name_of(&me))
                        .size(12.0)
                        .color(p.violet_light),
                );
                ui.label(small(" · ".into()));
            }
            ui.label(small(format!("{len} / {}", name::MAX)));
            if rule {
                let rule = aurora_i18n::tr!(
                    "ui-group-create-name-rule",
                    min = format!("{}", name::MIN),
                    max = format!("{}", name::MAX)
                );
                ui.label(egui::RichText::new(format!(" · {rule}")).size(12.0).color(p.amber));
            }
        });
        view::insignia(ui, p, insignia_rect, tex, &self.draft.name);
        // the insignia opens the texture picker, as Firestorm's: a photo
        // mark in its corner says so
        let mark = Rect::from_min_size(insignia_rect.right_bottom() - Vec2::splat(30.0), Vec2::splat(24.0));
        ui.painter()
            .rect_filled(mark, ui.visuals().widgets.inactive.corner_radius, p.panel.gamma_multiply(0.85));
        if let Some(t) = crate::ui::icons::global("photo-filled") {
            ui.painter().image(
                t.id(),
                Rect::from_center_size(mark.center(), Vec2::splat(14.0)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                p.ink,
            );
        }
        let pick = ui
            .interact(insignia_rect, egui::Id::unique("group_create_insignia"), Sense::click())
            .on_hover_text(aurora_i18n::tr!("ui-group-create-insignia-tip"))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if pick.clicked() && !self.sending {
            self.pick_insignia();
        }

        // the badges the group will have
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
        self.badges(&mut b, p);
        let used = b.min_rect().bottom().max(badges_top + 40.0);
        ui.allocate_rect(Rect::from_min_max(top, Pos2::new(banner_rect.right(), used)), Sense::hover());
        ui.add_space(10.0);

        // the identity's cards, the fees beside them (below in a narrow window)
        let side_w = 270.0;
        let wide = ui.available_width() >= 620.0;
        let draft = &mut self.draft;
        let mut change = false;
        egui::ScrollArea::vertical()
            .id_salt("group_create_body")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let enabled = !self.sending;
                let mut cards = |ui: &mut egui::Ui, c: &mut Ctx| {
                    // the insignia is chosen on the header's
                    let fields = IdentityFields {
                        ident: enabled,
                        options: enabled,
                        charter_limit: CHARTER_BYTES,
                        insignia_card: false,
                    };
                    change = administration::identity_cards(ui, c, &mut draft.info, Rating::Choice(&mut draft.rating), fields);
                    clip_bytes(&mut draft.info.charter, CHARTER_BYTES);
                };
                if wide {
                    ui.horizontal_top(|ui| {
                        let main_w = ui.available_width() - side_w - 10.0;
                        ui.vertical(|ui| {
                            ui.set_width(main_w);
                            cards(ui, c);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(side_w);
                            fees_card(ui, c);
                        });
                    });
                } else {
                    cards(ui, c);
                    ui.add_space(10.0);
                    fees_card(ui, c);
                }
            });
        if change {
            self.pick_insignia();
        }
    }

    /// The content (or "rating to choose" in amber), the enrollment, the fee.
    fn badges(&self, ui: &mut egui::Ui, p: &Palette) {
        let neutral = (p.muted, p.bar);
        match self.draft.rating {
            Some(moderate) => {
                let (rating, text) = if moderate {
                    (
                        aurora_i18n::tr!("common-moderate"),
                        aurora_i18n::tr!("ui-group-profile-moderate-content"),
                    )
                } else {
                    (
                        aurora_i18n::tr!("common-general"),
                        aurora_i18n::tr!("ui-group-profile-general-content"),
                    )
                };
                view::badge_with(ui, p, text, neutral.0, neutral.1, 13.0, |ui, r| {
                    widgets::paint_maturity_badge(ui.painter(), p, rating, r, widgets::inner_radius(ui.visuals()));
                });
            }
            None => {
                view::badge(
                    ui,
                    p,
                    "alert-triangle-filled",
                    aurora_i18n::tr!("ui-group-create-rating-to-choose"),
                    p.amber,
                    p.amber.gamma_multiply(0.14),
                );
            }
        }
        if self.draft.info.open_enrollment {
            view::badge(
                ui,
                p,
                "door-enter",
                aurora_i18n::tr!("ui-group-profile-open-enrollment"),
                neutral.0,
                neutral.1,
            );
        } else {
            view::badge(
                ui,
                p,
                "lock-filled",
                aurora_i18n::tr!("ui-group-profile-invitation-only"),
                neutral.0,
                neutral.1,
            );
        }
        let fee = self.draft.info.membership_fee;
        let text = if fee > 0 {
            aurora_i18n::tr!("ui-group-profile-fee", fee = aurora_i18n::integer(i64::from(fee)))
        } else {
            aurora_i18n::tr!("ui-group-profile-free").to_owned()
        };
        view::badge(ui, p, "coin-filled", &text, neutral.0, neutral.1);
        view::badge(
            ui,
            p,
            "check",
            aurora_i18n::tr!("ui-group-create-you-owner"),
            p.success,
            p.success.gamma_multiply(0.12),
        );
    }

    // ----------------------------------------------------------- classic look

    /// panel_group_creation_sidetray.xml.
    fn classic(&mut self, ui: &mut egui::Ui, c: &mut Ctx, actions: &mut Vec<GroupAction>) {
        let p = c.p;
        let enabled = !self.sending;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-create-classic-name"))
                    .size(12.5)
                    .color(p.ink),
            );
            let w = ui.available_width();
            self.name_edit(ui, None, w);
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("group_create_classic")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let insignia = self.draft.info.insignia;
                    if !insignia.is_nil() {
                        c.wanted_images.insert(insignia);
                    }
                    let pic = widgets::picture(
                        ui,
                        p,
                        (!insignia.is_nil()).then(|| c.images.get(&insignia)).flatten(),
                        Vec2::new(100.0, 110.0),
                        "users-group",
                        "",
                    )
                    .on_hover_text(aurora_i18n::tr!("ui-group-create-insignia-tip"));
                    if pic.clicked() && enabled {
                        self.pick_insignia();
                    }
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.draft.info.charter)
                                .hint_text(aurora_i18n::tr!("ui-group-create-charter-hint"))
                                .desired_rows(6)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    clip_bytes(&mut self.draft.info.charter, CHARTER_BYTES);
                });
                ui.add_space(6.0);
                ui.add_enabled_ui(enabled, |ui| {
                    widgets::checkbox(
                        ui,
                        p,
                        &mut self.draft.info.open_enrollment,
                        aurora_i18n::tr!("ui-group-create-anyone-can-join"),
                    )
                    .on_hover_text(aurora_i18n::tr!("ui-group-create-anyone-can-join-tip"));
                    let mut paid = self.draft.info.membership_fee > 0;
                    if widgets::checkbox(ui, p, &mut paid, aurora_i18n::tr!("ui-group-create-cost-to-join"))
                        .on_hover_text(aurora_i18n::tr!("ui-group-create-cost-to-join-tip"))
                        .changed()
                    {
                        self.draft.info.membership_fee = i32::from(paid);
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new("L$").size(12.5).color(p.muted));
                        ui.add_enabled(paid, egui::DragValue::new(&mut self.draft.info.membership_fee).range(0..=FEE_MAX));
                    });
                    let selected = match self.draft.rating {
                        None => aurora_i18n::tr!("ui-group-create-select-rating"),
                        Some(true) => aurora_i18n::tr!("ui-group-profile-moderate-content"),
                        Some(false) => aurora_i18n::tr!("ui-group-profile-general-content"),
                    };
                    egui::ComboBox::from_id_salt("group_create_rating")
                        .selected_text(selected)
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.rating, None, aurora_i18n::tr!("ui-group-create-select-rating"));
                            ui.selectable_value(
                                &mut self.draft.rating,
                                Some(true),
                                aurora_i18n::tr!("ui-group-profile-moderate-content"),
                            );
                            ui.selectable_value(
                                &mut self.draft.rating,
                                Some(false),
                                aurora_i18n::tr!("ui-group-profile-general-content"),
                            );
                        })
                        .response
                        .on_hover_text(aurora_i18n::tr!("ui-group-create-rating-tip"));
                });
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-create-fee-info"))
                        .size(11.5)
                        .color(p.muted),
                );
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(aurora_i18n::tr!("ui-group-create-more-info"))
                                .size(11.5)
                                .color(p.violet_light),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(MEMBERSHIP_URL));
                }
                ui.add_space(4.0);
                levels_list(ui, p, c.world, true);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let label = Self::create_label(c.world);
                    if widgets::action_button_if(ui, p, "", &label, ActionKind::Flat, Vec2::new(0.0, 24.0), enabled)
                        .on_hover_text(aurora_i18n::tr!("ui-group-create-create-tip"))
                        .clicked()
                    {
                        self.press(c.world, actions);
                    }
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, Vec2::new(0.0, 24.0))
                        .clicked()
                    {
                        self.open = false;
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-create-classic-note"))
                        .size(11.5)
                        .color(p.muted),
                );
            });
    }

    // ---------------------------------------------------------------- dialogs

    fn dialog(&mut self, ctx: &egui::Context, p: &Palette, world: &World, actions: &mut Vec<GroupAction>) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let title = aurora_i18n::tr!("ui-group-create-title");
        let text = match &dialog {
            Dialog::NameLength => aurora_i18n::tr!(
                "ui-group-create-name-length",
                min = format!("{}", name::MIN),
                max = format!("{}", name::MAX)
            ),
            Dialog::Mature => aurora_i18n::tr!("ui-group-create-moderate-question").to_owned(),
            Dialog::Cost(cost) => aurora_i18n::tr!("ui-group-create-cost", cost = aurora_i18n::integer(i64::from(*cost))),
            Dialog::Refused(message) => {
                let head = aurora_i18n::tr!("ui-group-create-refused");
                if message.trim().is_empty() {
                    head.to_owned()
                } else {
                    format!("{head}\n{}", message.trim())
                }
            }
            Dialog::TooManyGroups => aurora_i18n::tr!("ui-group-profile-too-many-groups").to_owned(),
        };
        let cost_label;
        let buttons: Vec<DialogButton> = match &dialog {
            Dialog::Mature => vec![
                DialogButton {
                    icon: "check",
                    label: aurora_i18n::tr!("common-yes"),
                    kind: ActionKind::Primary,
                    enabled: true,
                },
                DialogButton {
                    icon: "",
                    label: aurora_i18n::tr!("common-no"),
                    kind: ActionKind::Flat,
                    enabled: true,
                },
                DialogButton {
                    icon: "x",
                    label: aurora_i18n::tr!("common-cancel"),
                    kind: ActionKind::Flat,
                    enabled: true,
                },
            ],
            Dialog::Cost(cost) => {
                cost_label = aurora_i18n::tr!("ui-group-create-create-for", cost = aurora_i18n::integer(i64::from(*cost)));
                vec![
                    DialogButton {
                        icon: "plus",
                        label: &cost_label,
                        kind: ActionKind::Primary,
                        enabled: true,
                    },
                    DialogButton {
                        icon: "x",
                        label: aurora_i18n::tr!("common-cancel"),
                        kind: ActionKind::Flat,
                        enabled: true,
                    },
                ]
            }
            _ => vec![DialogButton {
                icon: "check",
                label: aurora_i18n::tr!("common-ok"),
                kind: ActionKind::Primary,
                enabled: true,
            }],
        };
        let mut clicked = None;
        let modal = widgets::Modal::new(egui::Id::unique("group_create_dialog")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new(title).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&text).size(12.5).color(p.muted));
            ui.add_space(12.0);
            clicked = widgets::dialog_buttons(ui, p, &buttons, 28.0);
        });
        let cancel = buttons.len() - 1;
        let choice = match clicked {
            Some(i) => i,
            None if modal.should_close() => cancel,
            None => return,
        };
        self.dialog = None;
        match (dialog, choice) {
            // confirmMatureApply: Yes / No set the rating, then go on
            (Dialog::Mature, 0 | 1) => {
                self.draft.rating = Some(choice == 0);
                self.press(world, actions);
            }
            (Dialog::Cost(_), 0) => self.send(actions),
            _ => {}
        }
    }
}

/// The fees by membership, ours marked (the creation panel's list).
fn levels_list(ui: &mut egui::Ui, p: &Palette, world: &World, classic: bool) {
    let levels = world.login.as_ref().map(|l| levels_of(&l.raw)).unwrap_or_default();
    if levels.is_empty() {
        let text = aurora_i18n::tr!("ui-group-create-fee-line", cost = aurora_i18n::integer(i64::from(fee(world))));
        ui.label(egui::RichText::new(text).size(12.5).color(p.ink));
        return;
    }
    for l in levels {
        ui.horizontal(|ui| {
            ui.set_min_height(20.0);
            let mut name = level_name(&l.package);
            if l.current {
                name = format!("{name} {}", aurora_i18n::tr!("ui-group-create-your-membership"));
            }
            // Firestorm's current row in yellow (DrYellow), Aurora's in the accent
            let ink = match (l.current, classic) {
                (true, true) => p.amber,
                (true, false) => p.ink,
                _ => p.muted,
            };
            let text = egui::RichText::new(name).size(12.5).color(ink);
            ui.label(if l.current { text.strong() } else { text });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cost = aurora_i18n::tr!("ui-group-create-amount", cost = aurora_i18n::integer(i64::from(l.cost)));
                let text = egui::RichText::new(cost).size(12.5).color(ink);
                ui.label(if l.current { text.strong() } else { text });
            });
        });
    }
}

/// The side card of the Aurora look: what the creation costs by
/// membership, our balance, the link to the memberships and the 48 hours
/// rule.
fn fees_card(ui: &mut egui::Ui, c: &mut Ctx) {
    let p = c.p;
    let (_, more) = view::card(
        ui,
        p,
        "coin-filled",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-create-fee-title"),
        Some(aurora_i18n::tr!("ui-group-create-more-info")),
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            levels_list(ui, p, c.world, false);
            if let Some(balance) = c.world.balance {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("ui-group-create-balance"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let short = balance < fee(c.world);
                        let text = aurora_i18n::tr!("ui-group-create-amount", cost = aurora_i18n::integer(i64::from(balance)));
                        ui.label(egui::RichText::new(text).size(12.0).color(if short { p.amber } else { p.muted }));
                    });
                });
            }
        },
    );
    if more {
        ui.ctx().open_url(egui::OpenUrl::new_tab(MEMBERSHIP_URL));
    }
    ui.add_space(10.0);
    view::card(
        ui,
        p,
        "alert-triangle-filled",
        p.amber,
        aurora_i18n::tr!("ui-group-create-members-title"),
        None,
        |ui| {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-create-members-note"))
                    .size(12.0)
                    .color(p.muted),
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn draft(name: &str, rating: Option<bool>) -> Draft {
        Draft {
            name: name.into(),
            rating,
            ..Default::default()
        }
    }

    #[test]
    fn create_asks_the_name_the_limit_the_rating_then_the_fee() {
        assert_eq!(next(&draft("abc", Some(false)), true, 100), Next::Ask(Dialog::NameLength));
        assert_eq!(next(&draft("Wolves", Some(false)), false, 100), Next::Ask(Dialog::TooManyGroups));
        assert_eq!(next(&draft("Wolves", None), true, 100), Next::Ask(Dialog::Mature));
        assert_eq!(next(&draft("Wolves", Some(true)), true, 100), Next::Ask(Dialog::Cost(100)));
        assert_eq!(next(&draft("  Wolves  ", Some(true)), true, 0), Next::Send);
    }

    #[test]
    fn the_fee_and_the_levels_come_from_the_login() {
        let raw = llsd_map! {
            "account_type" => "Premium",
            "account_level_benefits" => llsd_map! { "create_group_cost" => 50 },
            "premium_packages" => llsd_map! {
                "Premium" => llsd_map! { "benefits" => llsd_map! { "create_group_cost" => 50 } },
                "Base" => llsd_map! { "benefits" => llsd_map! { "create_group_cost" => 100 } },
            },
        };
        assert_eq!(fee_of(&raw), 50);
        let levels = levels_of(&raw);
        assert_eq!(
            levels,
            vec![
                Level {
                    package: "Base".into(),
                    cost: 100,
                    current: false
                },
                Level {
                    package: "Premium".into(),
                    cost: 50,
                    current: true
                },
            ]
        );
        // no benefits: free, as LLAgentBenefits off Second Life
        assert_eq!(fee_of(&Llsd::Undef), 0);
        assert!(levels_of(&Llsd::Undef).is_empty());
    }

    #[test]
    fn the_charter_keeps_511_bytes_on_a_character_boundary() {
        let mut s = "é".repeat(300);
        clip_bytes(&mut s, CHARTER_BYTES);
        assert_eq!(s.len(), 510);
        assert!(s.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_refusal_keeps_the_form_and_a_success_opens_the_group() {
        let mut ui = GroupCreateUi {
            open: true,
            draft: draft("Wolves", Some(false)),
            sending: true,
            ..Default::default()
        };
        assert_eq!(ui.on_reply(Uuid::nil(), false, "Name taken"), None);
        assert!(ui.open && !ui.sending);
        assert_eq!(ui.draft.name, "Wolves");
        assert_eq!(ui.dialog, Some(Dialog::Refused("Name taken".into())));
        let id = Uuid::from_u128(9);
        ui.sending = true;
        assert_eq!(ui.on_reply(id, true, ""), Some(id));
        assert!(!ui.open && !ui.sending);
    }
}
