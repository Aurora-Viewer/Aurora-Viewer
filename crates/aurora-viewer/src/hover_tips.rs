//! The world's hover tips: the object, avatar or parcel under the pointer,
//! shown once the pointer rests on it (Preferences › Interface; drawn by
//! `ui/hover_tips.rs`, run each frame by `app/hover_tips.rs`).
//!
//! Port of LLToolPie::handleToolTip, handleTooltipObject and
//! handleTooltipLand (indra/newview/lltoolpie.cpp), of LLToolTipMgr's
//! timing (indra/llui/lltooltip.cpp) and of LLViewerParcelMgr::
//! setHoverParcel (indra/newview/llviewerparcelmgr.cpp), originally LGPL 2.1.
//! The texts follow the Legacy Viewer's advanced tooltips (FSAdvancedTooltips):
//! description, owner, flags, prims and land impact, position, distance.

use crate::settings::HoverTips;
use crate::world::World;
use aurora_net::build::{ObjectProps, perm};
use aurora_net::{ParcelInfo, RegionHandle, parcel_flags as pf};
use egui::{Pos2, Rect};
use glam::{DVec3, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Update flags of an object (llprimitive/object_flags.h).
pub mod flag {
    pub const USE_PHYSICS: u32 = 1 << 0;
    pub const SCRIPTED: u32 = 1 << 6;
    pub const HANDLE_TOUCH: u32 = 1 << 7;
    pub const TAKES_MONEY: u32 = 1 << 9;
    pub const PHANTOM: u32 = 1 << 10;
    pub const ALLOW_INVENTORY_DROP: u32 = 1 << 16;
    pub const TEMPORARY_ON_REZ: u32 = 1 << 29;
}

/// What lies under the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Subject {
    /// A prim (`idx`) of the linkset whose root is `root`.
    Object { idx: usize, root: usize, root_id: Uuid },
    /// An avatar, or something it wears (LLToolPie shows the wearer).
    Avatar { idx: usize, id: Uuid },
    /// The ground at `local` (region coordinates) of region `handle`.
    Land {
        handle: RegionHandle,
        local: Vec3,
        origin: (u32, u32),
    },
}

/// What makes two subjects the same tip: the linkset, the avatar, the parcel
/// (its 4 m cell until the parcel is known).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Object(Uuid),
    Avatar(Uuid),
    Parcel(RegionHandle, i32),
    Cell(RegionHandle, u32, u32),
}

/// The subject of an object slot (`idx`) or the ground (`None`) under the
/// pointer, at the world point `point`. HUDs never get here (the hover of
/// a HUD comes first and shows no tip, as in handleTooltipObject).
pub fn subject_at(world: &World, idx: Option<usize>, point: Vec3) -> Option<Subject> {
    let Some(idx) = idx else {
        let (gx, gy) = crate::ui::minimap::to_global(world, point);
        let (handle, local, _) = crate::ui::land::LandUi::region_at_global(world, gx, gy)?;
        return Some(Subject::Land {
            handle,
            local,
            origin: aurora_net::handle_to_origin(handle),
        });
    };
    let o = world.objects.get(idx)?;
    if o.is_avatar() {
        return Some(Subject::Avatar { idx, id: o.full_id });
    }
    if let Some(id) = crate::ui::context::wearer(world, idx) {
        let avatar = world.objects.index_of_uuid(&id)?;
        return Some(Subject::Avatar { idx: avatar, id });
    }
    let root = root_of(world, idx);
    Some(Subject::Object {
        idx,
        root,
        root_id: world.objects.get(root)?.full_id,
    })
}

/// The root of a rezzed linkset.
fn root_of(world: &World, idx: usize) -> usize {
    let mut i = idx;
    for _ in 0..64 {
        let Some(o) = world.objects.get(i) else { break };
        match world.objects.parent_of(o) {
            Some(p) if world.objects.get(p).is_some_and(|po| !po.is_avatar()) => i = p,
            _ => break,
        }
    }
    i
}

impl Subject {
    pub fn key(&self, parcels: &HoverParcels) -> Key {
        match *self {
            Subject::Object { root_id, .. } => Key::Object(root_id),
            Subject::Avatar { id, .. } => Key::Avatar(id),
            Subject::Land { handle, local, origin } => match parcels.at(handle, local, origin) {
                Some(p) => Key::Parcel(handle, p.local_id),
                None => Key::Cell(handle, (local.x / CELL) as u32, (local.y / CELL) as u32),
            },
        }
    }
}

/// The parcel grid step (PARCEL_GRID_STEP_METERS).
const CELL: f32 = 4.0;

/// The parcels the hover asked about, and the request of the one under the
/// pointer (LLViewerParcelMgr::setHoverParcel: one ParcelPropertiesRequest
/// for the 4 m cell, with HOVERED_PARCEL_SEQ_ID, only when the pointer
/// leaves the last parcel known).
#[derive(Default)]
pub struct HoverParcels {
    parcels: Vec<(RegionHandle, Arc<ParcelInfo>)>,
    /// The cell last asked, and when.
    asked: Option<(RegionHandle, u32, u32, Instant)>,
}

/// Parcels kept: those under the pointer lately.
const KEPT_PARCELS: usize = 16;
/// A cell asked again after this long without an answer.
const PARCEL_RETRY: Duration = Duration::from_secs(5);

impl HoverParcels {
    /// The known parcel holding `local` of region `handle`.
    pub fn at(&self, handle: RegionHandle, local: Vec3, origin: (u32, u32)) -> Option<&Arc<ParcelInfo>> {
        let pos = DVec3::new(
            f64::from(origin.0) + f64::from(local.x),
            f64::from(origin.1) + f64::from(local.y),
            0.0,
        );
        self.parcels
            .iter()
            .rev()
            .find(|(h, p)| *h == handle && crate::world::landmarks::in_parcel(&p.bitmap, origin, pos))
            .map(|(_, p)| p)
    }

    /// The request for the parcel at `local`, when it is not known and its
    /// cell was not just asked.
    pub fn request(&mut self, handle: RegionHandle, local: Vec3, origin: (u32, u32), now: Instant) -> Option<aurora_net::NetCommand> {
        if self.at(handle, local, origin).is_some() {
            return None;
        }
        let (cx, cy) = ((local.x / CELL).max(0.0) as u32, (local.y / CELL).max(0.0) as u32);
        if self
            .asked
            .is_some_and(|(h, x, y, at)| h == handle && x == cx && y == cy && now.duration_since(at) < PARCEL_RETRY)
        {
            return None;
        }
        self.asked = Some((handle, cx, cy, now));
        // a rectangle around the point: the parcel sent back holds it
        let (west, south) = (cx as f32 * CELL, cy as f32 * CELL);
        Some(aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::ParcelRequest {
            handle,
            sequence: aurora_net::land::HOVERED_PARCEL_SEQ_ID,
            snap: false,
            west,
            south,
            east: west + CELL,
            north: south + CELL,
        }))
    }

    /// ParcelProperties of the hovered parcel (LLViewerParcelMgr::
    /// processParcelProperties, HOVERED_PARCEL_SEQ_ID).
    pub fn on_parcel(&mut self, handle: RegionHandle, parcel: Arc<ParcelInfo>) {
        self.parcels.retain(|(h, p)| !(*h == handle && p.local_id == parcel.local_id));
        self.parcels.push((handle, parcel));
        if self.parcels.len() > KEPT_PARCELS {
            self.parcels.remove(0);
        }
        self.asked = None;
    }
}

/// How a tip is drawn this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    pub subject: Subject,
    pub key: Key,
    /// The pointer when the tip appeared: the bubble and the card stay
    /// there while the pointer moves within the subject.
    pub anchor: Pos2,
}

/// LLToolTipMgr's rules for one tip: it appears once the pointer rested
/// `delay` on the same subject; a press, the wheel or a key hides it until
/// the pointer leaves the place where it was; the pointer may travel to the
/// tip and rest on it (the card's buttons) without losing it.
#[derive(Default)]
pub struct Timing {
    /// The subject under the pointer and since when the pointer rests.
    pending: Option<(Key, Instant)>,
    shown: Option<Shown>,
    /// Hidden by a press: no tip until the pointer leaves `near`.
    blocked: Option<Rect>,
    last_pointer: Option<Pos2>,
}

/// Room around the point a tip appeared at where the pointer may wander
/// (LLToolTipMgr's mouse near rect).
const NEAR: f32 = 12.0;

/// One frame of the pointer.
pub struct Frame {
    /// What the world hover found, None over nothing (sky, a window).
    pub subject: Option<(Subject, Key)>,
    pub pointer: Option<Pos2>,
    /// A button pressed, the wheel turned or a key pressed this frame.
    pub interrupted: bool,
    /// Where the tip was drawn last frame (the pointer may rest there).
    pub tip_rect: Option<Rect>,
    /// The tip follows the pointer's subject at once once shown (the
    /// readout); the bubble and the card stay where they appeared.
    pub follows: bool,
    pub delay: Duration,
    pub now: Instant,
}

impl Timing {
    pub fn hide(&mut self) {
        self.shown = None;
        self.pending = None;
    }

    /// The rect around the tip and the point it came from, where the
    /// pointer keeps it.
    fn near(shown: &Shown, tip: Option<Rect>) -> Rect {
        let around = Rect::from_center_size(shown.anchor, egui::vec2(NEAR * 2.0, NEAR * 2.0));
        tip.map_or(around, |t| t.expand(4.0).union(around))
    }

    pub fn update(&mut self, f: Frame) -> Option<Shown> {
        let moved = f.pointer != self.last_pointer;
        self.last_pointer = f.pointer;
        let Some(pointer) = f.pointer else {
            self.hide();
            return None;
        };
        if f.interrupted {
            let near = self.shown.map_or_else(
                || Rect::from_center_size(pointer, egui::vec2(NEAR * 2.0, NEAR * 2.0)),
                |s| Self::near(&s, f.tip_rect),
            );
            // a press on the card itself is its button's
            if !f.tip_rect.is_some_and(|t| t.contains(pointer)) {
                self.blocked = Some(near);
                self.hide();
                return None;
            }
        }
        if let Some(near) = self.blocked {
            if near.contains(pointer) {
                return None;
            }
            self.blocked = None;
        }
        if let Some(shown) = self.shown {
            let key = f.subject.map(|(_, k)| k);
            if key == Some(shown.key) {
                if f.follows {
                    self.shown = f.subject.map(|(subject, key)| Shown {
                        subject,
                        key,
                        anchor: pointer,
                    });
                }
                return self.shown;
            }
            // on its way to the tip, or resting on it
            if !f.follows && Self::near(&shown, f.tip_rect).contains(pointer) {
                return self.shown;
            }
            self.shown = None;
            if f.follows
                && let Some((subject, key)) = f.subject
            {
                // the readout changes with the pointer, without a new delay
                self.shown = Some(Shown {
                    subject,
                    key,
                    anchor: pointer,
                });
                return self.shown;
            }
        }
        let Some((subject, key)) = f.subject else {
            self.pending = None;
            return None;
        };
        match self.pending {
            Some((k, _)) if k == key && !moved => {}
            Some((k, since)) if k == key && f.follows => self.pending = Some((k, since)),
            _ => self.pending = Some((key, f.now)),
        }
        let (_, since) = self.pending?;
        if f.now.duration_since(since) >= f.delay {
            self.shown = Some(Shown {
                subject,
                key,
                anchor: pointer,
            });
            self.pending = None;
        }
        self.shown
    }
}

/// A flag of the subject, shown as a chip with its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Pay,
    Touch,
    Script,
    Physics,
    DropInventory,
    Phantom,
    Temporary,
    NoFly,
    GroupBuild,
    NoBuild,
    NoTerraform,
    Damage,
    GroupScripts,
    NoScripts,
}

impl Flag {
    pub fn label(self) -> &'static str {
        match self {
            Flag::Pay => aurora_i18n::tr!("hover-tips-flag-pay"),
            Flag::Touch => aurora_i18n::tr!("hover-tips-flag-touch"),
            Flag::Script => aurora_i18n::tr!("hover-tips-flag-script"),
            Flag::Physics => aurora_i18n::tr!("hover-tips-flag-physics"),
            Flag::DropInventory => aurora_i18n::tr!("hover-tips-flag-drop-inventory"),
            Flag::Phantom => aurora_i18n::tr!("hover-tips-flag-phantom"),
            Flag::Temporary => aurora_i18n::tr!("hover-tips-flag-temporary"),
            Flag::NoFly => aurora_i18n::tr!("hover-tips-flag-no-fly"),
            Flag::GroupBuild => aurora_i18n::tr!("hover-tips-flag-group-build"),
            Flag::NoBuild => aurora_i18n::tr!("hover-tips-flag-no-build"),
            Flag::NoTerraform => aurora_i18n::tr!("hover-tips-flag-no-terraform"),
            Flag::Damage => aurora_i18n::tr!("hover-tips-flag-damage"),
            Flag::GroupScripts => aurora_i18n::tr!("hover-tips-flag-group-scripts"),
            Flag::NoScripts => aurora_i18n::tr!("hover-tips-flag-no-scripts"),
        }
    }

    /// The Legacy Viewer's word for the flag (its tooltip line).
    fn legacy(self) -> &'static str {
        match self {
            Flag::Pay => aurora_i18n::tr!("hover-tips-legacy-pay"),
            Flag::Touch => aurora_i18n::tr!("hover-tips-legacy-touch"),
            Flag::Script => aurora_i18n::tr!("hover-tips-legacy-script"),
            Flag::Physics => aurora_i18n::tr!("hover-tips-legacy-physics"),
            Flag::DropInventory => aurora_i18n::tr!("hover-tips-legacy-drop-inventory"),
            Flag::Phantom => aurora_i18n::tr!("hover-tips-legacy-phantom"),
            Flag::Temporary => aurora_i18n::tr!("hover-tips-legacy-temporary"),
            Flag::NoFly => aurora_i18n::tr!("hover-tips-legacy-no-fly"),
            Flag::GroupBuild => aurora_i18n::tr!("hover-tips-legacy-group-build"),
            Flag::NoBuild => aurora_i18n::tr!("hover-tips-legacy-no-build"),
            Flag::NoTerraform => aurora_i18n::tr!("hover-tips-legacy-no-terraform"),
            Flag::Damage => aurora_i18n::tr!("hover-tips-legacy-damage"),
            Flag::GroupScripts => aurora_i18n::tr!("hover-tips-legacy-group-scripts"),
            Flag::NoScripts => aurora_i18n::tr!("hover-tips-legacy-no-scripts"),
        }
    }

    /// Its Tabler icon: the one of the action when it has one.
    pub fn icon(self) -> &'static str {
        match self {
            Flag::Pay => "coin-filled",
            Flag::Touch => "hand-finger",
            Flag::Script => "code",
            Flag::Physics => "ball-bowling-filled",
            Flag::DropInventory => "package-import",
            Flag::Phantom => "ghost-filled",
            Flag::Temporary => "hourglass-filled",
            Flag::NoFly => "plane-off",
            Flag::GroupBuild | Flag::NoBuild => "hammer-off",
            Flag::NoTerraform => "shovel",
            Flag::Damage => "alert-triangle-filled",
            Flag::GroupScripts | Flag::NoScripts => "code-off",
        }
    }

    /// The card has a button for it: the chip would say it twice.
    pub fn has_action(self) -> bool {
        matches!(self, Flag::Pay | Flag::Touch)
    }
}

/// What a tip's buttons do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TipAction {
    /// Buy the object (its root), pay or touch the prim under the pointer.
    Buy(Uuid),
    Pay(Uuid),
    Touch(u32),
    /// The window of the object's prims (the Legacy Viewer's object inspector).
    Inspect(Uuid),
    Profile(Uuid),
    Im(Uuid),
    PayResident(Uuid),
    /// About Land / Buy Land on the parcel at a world point.
    AboutLand(Vec3),
    BuyLand(Vec3),
}

impl TipAction {
    pub fn label(self) -> &'static str {
        match self {
            TipAction::Buy(_) => aurora_i18n::tr!("hover-tips-buy"),
            TipAction::Pay(_) | TipAction::PayResident(_) => aurora_i18n::tr!("hover-tips-pay"),
            TipAction::Touch(_) => aurora_i18n::tr!("hover-tips-touch"),
            TipAction::Inspect(_) => aurora_i18n::tr!("hover-tips-inspect"),
            TipAction::Profile(_) => aurora_i18n::tr!("hover-tips-profile"),
            TipAction::Im(_) => aurora_i18n::tr!("hover-tips-message"),
            TipAction::AboutLand(_) => aurora_i18n::tr!("hover-tips-about-land"),
            TipAction::BuyLand(_) => aurora_i18n::tr!("hover-tips-buy-land"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            TipAction::Buy(_) | TipAction::BuyLand(_) => "shopping-cart-filled",
            TipAction::Pay(_) | TipAction::PayResident(_) => "coin-filled",
            TipAction::Touch(_) => "hand-finger",
            TipAction::Inspect(_) | TipAction::AboutLand(_) => "info-circle-filled",
            TipAction::Profile(_) => "id-filled",
            TipAction::Im(_) => "message-filled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipKind {
    Object,
    Avatar,
    Land,
}

/// Everything a tip can show; each form picks its part.
#[derive(Debug, Clone, PartialEq)]
pub struct Tip {
    pub kind: TipKind,
    /// The line above the title: the avatar's group title, "Land".
    pub kicker: Option<String>,
    pub title: String,
    /// The line below the title: the avatar's username.
    pub sub: Option<String>,
    /// The owner, and whether it is a group.
    pub owner: Option<(String, bool)>,
    /// "For sale · L$ 250" and what is sold or the parcel's area.
    pub sale: Option<(String, String)>,
    pub desc: Option<String>,
    /// The card's lines of facts.
    pub rows: Vec<(&'static str, String)>,
    pub flags: Vec<Flag>,
    /// The card's buttons, and its "i".
    pub actions: Vec<TipAction>,
    pub inspect: TipAction,
    /// The Legacy Viewer's text (its tooltip, with the "i" for objects and
    /// avatars).
    pub legacy: String,
}

/// LLSaleInfo: 1 original, 2 copy, 3 contents.
fn sale_type(t: u8) -> &'static str {
    match t {
        1 => aurora_i18n::tr!("common-original"),
        3 => aurora_i18n::tr!("common-contents"),
        _ => aurora_i18n::tr!("common-copy"),
    }
}

fn lindens(n: i32) -> String {
    aurora_i18n::tr!("ui-group-land-lindens", amount = aurora_i18n::integer(i64::from(n)))
}

/// for_sale_selection (llviewermenu.cpp): for sale, transferable, and
/// copyable when a copy is sold.
pub fn for_sale(p: &ObjectProps) -> bool {
    (1..=3).contains(&p.sale_type) && p.owner_mask & perm::TRANSFER != 0 && (p.owner_mask & perm::COPY != 0 || p.sale_type != 2)
}

/// needs_tooltip (lltoolpie.cpp): without "on all objects", only what can
/// be touched, paid, given items or clicked, or is copyable or for sale,
/// or has media.
fn interactive(world: &World, idx: usize, props: Option<&ObjectProps>) -> bool {
    let flags = |i: usize| world.objects.get(i).map_or(0, |o| o.update_flags);
    let parent = world.objects.get(idx).and_then(|o| world.objects.parent_of(o)).map_or(0, flags);
    let own = flags(idx);
    if (own | parent) & (flag::HANDLE_TOUCH | flag::TAKES_MONEY) != 0 || own & flag::ALLOW_INVENTORY_DROP != 0 {
        return true;
    }
    if crate::interaction::final_action(world, idx).is_some_and(|a| a != crate::interaction::code::NONE) {
        return true;
    }
    if world.objects.get(idx).is_some_and(|o| !o.media_url.is_empty()) {
        return true;
    }
    props.is_some_and(|p| p.everyone_mask & perm::COPY != 0 || for_sale(p))
}

/// A resident's or a group's name, asked when missing.
fn owner_name(world: &mut World, id: Uuid, group: bool) -> Option<String> {
    if group {
        let groups = world.groups.groups.clone();
        return world.land.group_name(&id, &groups);
    }
    world.social.want_name(id);
    let name = world.social.name_of(&id);
    (name != "…").then_some(name)
}

/// What else the tips need from the app: the properties of the hovered
/// objects (ObjectPropertiesFamily) and their land impact (GetObjectCost).
pub struct Sources<'a> {
    pub props: &'a HashMap<Uuid, ObjectProps>,
    pub parcels: &'a HoverParcels,
    pub land_impact: Option<f32>,
    /// The world point under the pointer, and where the agent stands.
    pub point: Vec3,
    pub agent: Vec3,
}

/// The tip of `subject`, or None when it shows none (an object that is not
/// interactive without "on all objects", land without "on land", a parcel
/// not known yet).
pub fn build(world: &mut World, subject: Subject, s: &Sources, settings: &HoverTips) -> Option<Tip> {
    match subject {
        Subject::Object { idx, root, root_id } => object_tip(world, idx, root, root_id, s, settings),
        Subject::Avatar { idx, id } => Some(avatar_tip(world, idx, id, s, settings)),
        Subject::Land { handle, local, origin } => {
            if !settings.land {
                return None;
            }
            let parcel = s.parcels.at(handle, local, origin)?.clone();
            Some(land_tip(world, &parcel, s.point))
        }
    }
}

fn object_tip(world: &mut World, idx: usize, root: usize, root_id: Uuid, s: &Sources, settings: &HoverTips) -> Option<Tip> {
    let props = s.props.get(&root_id);
    if !settings.all_objects && !interactive(world, idx, props) {
        return None;
    }
    let (prim, root_o) = (world.objects.get(idx)?, world.objects.get(root)?);
    let (prim_flags, root_flags) = (prim.update_flags, root_o.update_flags);
    let parent_flags = world
        .objects
        .parent_of(prim)
        .and_then(|p| world.objects.get(p))
        .map_or(0, |p| p.update_flags);
    let (local_id, prim_id, region) = (prim.key.local_id, prim.full_id, root_o.key.region);
    let title = props
        .map(|p| p.name.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if props.is_some() {
                aurora_i18n::tr!("hover-tips-no-name").to_owned()
            } else {
                aurora_i18n::tr!("common-loading").to_owned()
            }
        });
    // "(No Description)" is the simulator's empty description
    let desc = props
        .map(|p| p.description.trim())
        .filter(|d| !d.is_empty() && *d != "(No Description)")
        .map(str::to_owned);
    let owner = props.and_then(|p| {
        let group = p.owner_id.is_nil() && !p.group_id.is_nil() || (p.owner_id == p.group_id && !p.group_id.is_nil());
        let id = if p.owner_id.is_nil() { p.group_id } else { p.owner_id };
        (!id.is_nil()).then_some((id, group))
    });
    let owner = owner.map(|(id, group)| {
        let name = owner_name(world, id, group).unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
        (name, group)
    });
    let sale = props
        .filter(|p| for_sale(p))
        .map(|p| (sale_line(p.sale_price), sale_type(p.sale_type).to_owned()));
    let mut flags = Vec::new();
    // the Legacy Viewer's order: script, physics, touch, L$, drop, phantom, temporary
    for (on, f) in [
        (prim_flags & flag::SCRIPTED != 0, Flag::Script),
        (prim_flags & flag::USE_PHYSICS != 0, Flag::Physics),
        ((prim_flags | parent_flags) & flag::HANDLE_TOUCH != 0, Flag::Touch),
        ((prim_flags | parent_flags) & flag::TAKES_MONEY != 0, Flag::Pay),
        (prim_flags & flag::ALLOW_INVENTORY_DROP != 0, Flag::DropInventory),
        (prim_flags & flag::PHANTOM != 0, Flag::Phantom),
        (prim_flags & flag::TEMPORARY_ON_REZ != 0, Flag::Temporary),
    ] {
        if on {
            flags.push(f);
        }
    }
    let mut actions = Vec::new();
    if sale.is_some() {
        actions.push(TipAction::Buy(prim_id));
    }
    if (prim_flags | parent_flags | root_flags) & flag::TAKES_MONEY != 0 {
        actions.push(TipAction::Pay(prim_id));
    }
    if (prim_flags | parent_flags) & flag::HANDLE_TOUCH != 0 {
        actions.push(TipAction::Touch(local_id));
    }
    let prims = crate::build::family(world, root).len();
    let position = crate::scene::Scene::object_transform(world, root, Instant::now(), 0).map(|xf| xf.0);
    let region_pos = world.region_offset(region).zip(position).map(|(offset, p)| p - offset);
    let distance = position.map(|p| p.distance(s.agent));
    let impact = s.land_impact;
    let mut rows = Vec::new();
    if let Some((name, _)) = &owner {
        rows.push((aurora_i18n::tr!("common-owner"), name.clone()));
    }
    if let Some(p) = props.filter(|p| for_sale(p)) {
        rows.push((
            aurora_i18n::tr!("hover-tips-sale"),
            format!("{} · {}", lindens(p.sale_price), sale_type(p.sale_type)),
        ));
    }
    rows.push((aurora_i18n::tr!("hover-tips-impact"), impact_text(prims, impact)));
    if let Some(p) = region_pos {
        rows.push((aurora_i18n::tr!("hover-tips-position"), position_text(p)));
    }
    if let Some(d) = distance {
        rows.push((aurora_i18n::tr!("common-distance"), meters(d)));
    }
    let legacy = legacy_object(
        &title,
        desc.as_deref(),
        owner.as_ref(),
        props.filter(|p| for_sale(p)),
        &flags,
        prims,
        impact,
        region_pos,
        distance,
    );
    Some(Tip {
        kind: TipKind::Object,
        kicker: None,
        title,
        sub: None,
        owner,
        sale,
        desc,
        rows,
        flags,
        actions,
        inspect: TipAction::Inspect(root_id),
        legacy,
    })
}

/// "For sale · L$ 250".
fn sale_line(price: i32) -> String {
    aurora_i18n::tr!("hover-tips-for-sale", price = lindens(price))
}

/// "3 prims · 2 LI", or the prims alone while the impact is unknown.
fn impact_text(prims: usize, impact: Option<f32>) -> String {
    let prims = aurora_i18n::tr!("hover-tips-prims", count = prims as i64);
    match impact {
        Some(li) => format!(
            "{prims} · {}",
            aurora_i18n::tr!("hover-tips-land-impact", count = li.round() as i64)
        ),
        None => prims,
    }
}

/// "118, 96, 23".
fn position_text(p: Vec3) -> String {
    format!("{}, {}, {}", p.x.round() as i64, p.y.round() as i64, p.z.round() as i64)
}

/// "9,4 m".
fn meters(d: f32) -> String {
    format!("{} m", aurora_i18n::number(f64::from(d), 1))
}

#[allow(clippy::too_many_arguments)]
fn legacy_object(
    title: &str,
    desc: Option<&str>,
    owner: Option<&(String, bool)>,
    sale: Option<&ObjectProps>,
    flags: &[Flag],
    prims: usize,
    impact: Option<f32>,
    position: Option<Vec3>,
    distance: Option<f32>,
) -> String {
    // "L$250: name" for an object for sale (TooltipPrice)
    let mut text = match sale {
        Some(p) => aurora_i18n::tr!("hover-tips-legacy-price", price = p.sale_price.to_string(), name = title.to_owned()),
        None => title.to_owned(),
    };
    if let Some(d) = desc {
        text.push('\n');
        text.push_str(d);
    }
    if let Some((name, _)) = owner {
        text.push('\n');
        text.push_str(&aurora_i18n::tr!("hover-tips-legacy-owner", name = name.clone()));
    }
    if !flags.is_empty() {
        text.push('\n');
        text.push_str(&flags.iter().map(|f| f.legacy()).collect::<Vec<_>>().join(" "));
    }
    text.push_str("\n\n");
    text.push_str(&aurora_i18n::tr!("hover-tips-legacy-prims", count = prims as i64));
    match impact {
        Some(li) => text.push_str(&aurora_i18n::tr!("hover-tips-legacy-impact", count = li.round() as i64)),
        None => text.push_str(aurora_i18n::tr!("hover-tips-legacy-impact-loading")),
    }
    if let Some(p) = position {
        text.push('\n');
        text.push_str(&aurora_i18n::tr!(
            "hover-tips-legacy-position",
            position = format!("<{:.2}, {:.2}, {:.2}>", p.x, p.y, p.z)
        ));
    }
    if let Some(d) = distance {
        text.push('\n');
        text.push_str(&aurora_i18n::tr!("hover-tips-legacy-distance", distance = format!("{d:.2}")));
    }
    text
}

fn avatar_tip(world: &mut World, idx: usize, id: Uuid, s: &Sources, settings: &HoverTips) -> Tip {
    world.social.want_name(id);
    let names = &world.social.avatar_names;
    let known = names.get(&id).cloned();
    let title = match &known {
        Some(n) => n.display(&names.options),
        None => world
            .avatar_name(&id)
            .unwrap_or_else(|| aurora_i18n::tr!("hover-tips-person").to_owned()),
    };
    let sub = known
        .as_ref()
        .filter(|n| !n.is_default && !n.username.is_empty() && n.username != title)
        .map(|n| n.username.clone());
    let group_title = settings
        .group_titles
        .then(|| world.objects.get(idx).and_then(|o| o.group_title()))
        .flatten()
        .filter(|t| !t.trim().is_empty());
    let position = world
        .objects
        .get(idx)
        .filter(|o| o.parent_id == 0)
        .and_then(|_| crate::scene::Scene::object_transform(world, idx, Instant::now(), 0))
        .map_or(s.point, |xf| xf.0);
    let distance = position.distance(s.agent);
    let ours = id == world.agent_id;
    let actions = if ours {
        Vec::new()
    } else {
        vec![TipAction::Im(id), TipAction::PayResident(id)]
    };
    // getCompleteName, with the group title above it (FSShowGroupTitleInTooltip)
    let complete = known.as_ref().map(|n| n.complete(&names.options)).unwrap_or_else(|| title.clone());
    let legacy = match &group_title {
        Some(t) => format!("{t}\n{complete}"),
        None => complete,
    };
    Tip {
        kind: TipKind::Avatar,
        kicker: group_title,
        title,
        sub,
        owner: None,
        sale: None,
        desc: None,
        rows: vec![(aurora_i18n::tr!("common-distance"), meters(distance))],
        flags: Vec::new(),
        actions,
        inspect: TipAction::Profile(id),
        legacy,
    }
}

/// The parcel's restrictions, as handleTooltipLand lists them (in the order
/// of the About Land checkboxes); none on our own land, where everything is
/// allowed.
pub fn land_flags(p: &ParcelInfo) -> Vec<Flag> {
    let f = p.flags;
    let mut out = Vec::new();
    if f & pf::CREATE_OBJECTS == 0 {
        out.push(if f & pf::CREATE_GROUP_OBJECTS != 0 {
            Flag::GroupBuild
        } else {
            Flag::NoBuild
        });
    }
    if f & pf::ALLOW_TERRAFORM == 0 {
        out.push(Flag::NoTerraform);
    }
    if f & pf::ALLOW_DAMAGE != 0 {
        out.push(Flag::Damage);
    }
    if f & pf::ALLOW_FLY == 0 {
        out.push(Flag::NoFly);
    }
    if f & pf::ALLOW_OTHER_SCRIPTS == 0 {
        out.push(if f & pf::ALLOW_GROUP_SCRIPTS != 0 {
            Flag::GroupScripts
        } else {
            Flag::NoScripts
        });
    }
    out
}

fn land_tip(world: &mut World, p: &ParcelInfo, point: Vec3) -> Tip {
    let public = p.owner_id.is_nil();
    let owner = if public {
        (aurora_i18n::tr!("hover-tips-public").to_owned(), false)
    } else {
        let name = owner_name(world, p.owner_id, p.is_group_owned).unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
        (name, p.is_group_owned)
    };
    let ours = p.owner_id == world.agent_id;
    let flags = if ours { Vec::new() } else { land_flags(p) };
    let for_sale = p.flags & pf::FOR_SALE != 0;
    let area = aurora_i18n::tr!("ui-group-profile-area", area = aurora_i18n::integer(i64::from(p.area)));
    let sale = for_sale.then(|| (sale_line(p.sale_price), area.clone()));
    let title = if p.name.trim().is_empty() {
        aurora_i18n::tr!("hover-tips-no-name").to_owned()
    } else {
        p.name.trim().to_owned()
    };
    let mut rows = vec![
        (aurora_i18n::tr!("common-owner"), owner.0.clone()),
        (aurora_i18n::tr!("hover-tips-area"), area),
    ];
    if for_sale {
        let per_m2 = if p.area > 0 {
            format!(
                " · {}",
                aurora_i18n::tr!(
                    "hover-tips-per-m2",
                    price = aurora_i18n::number(f64::from(p.sale_price) / f64::from(p.area), 1)
                )
            )
        } else {
            String::new()
        };
        rows.push((aurora_i18n::tr!("hover-tips-sale"), format!("{}{per_m2}", lindens(p.sale_price))));
    }
    // the sale may be set to one buyer (auth_buyer)
    let may_buy = for_sale && !ours && (p.auth_buyer.is_nil() || p.auth_buyer == world.agent_id);
    let actions = if may_buy { vec![TipAction::BuyLand(point)] } else { Vec::new() };
    let mut legacy = aurora_i18n::tr!("hover-tips-legacy-land", name = p.name.clone());
    legacy.push('\n');
    legacy.push_str(&aurora_i18n::tr!(
        "hover-tips-legacy-owner",
        name = if owner.1 {
            aurora_i18n::tr!("hover-tips-legacy-group", name = owner.0.clone())
        } else {
            owner.0.clone()
        }
    ));
    if !flags.is_empty() {
        legacy.push('\n');
        legacy.push_str(&flags.iter().map(|f| f.legacy()).collect::<Vec<_>>().join(", "));
    }
    if for_sale {
        legacy.push('\n');
        legacy.push_str(&aurora_i18n::tr!("hover-tips-legacy-for-sale", price = p.sale_price.to_string()));
    }
    Tip {
        kind: TipKind::Land,
        kicker: Some(aurora_i18n::tr!("hover-tips-land").to_owned()),
        title,
        sub: None,
        owner: Some(owner),
        sale,
        desc: Some(p.desc.trim().to_owned()).filter(|d| !d.is_empty()),
        rows,
        flags,
        actions,
        inspect: TipAction::AboutLand(point),
        legacy,
    }
}

#[cfg(test)]
mod tests;
