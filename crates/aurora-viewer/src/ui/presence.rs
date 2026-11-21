//! The status of a resident and the color of its dot, shared by the name
//! plates and the Conversations window so that both say the same thing.
//! What the viewer really knows, in this order:
//!
//! 1. A resident in one of our regions (in the coarse locations) is online,
//!    away or in do not disturb from the animations their avatar plays
//!    (ANIM_AGENT_AWAY / ANIM_AGENT_DO_NOT_DISTURB): the source of the name
//!    plates (`ui/hud.rs`), as of Firestorm's tags
//!    (LLVOAvatar::idleUpdateNameTagText).
//! 2. Elsewhere, a friend who lets us see their online status is online or
//!    offline (LLAvatarTracker::isBuddyOnline).
//! 3. Otherwise, a resident whose last word was an automatic response
//!    (IM_DO_NOT_DISTURB_AUTO_RESPONSE) reads as do not disturb until their
//!    next ordinary message. Firestorm sends that same dialog for its
//!    autorespond modes and its rejected teleport offers or friendship
//!    requests: an automatic response is a hint, not a proof.
//! 4. Anyone else is unknown.

use crate::theme::Palette;
use crate::world::World;
use crate::world::names::NameScope;
use crate::world::social::rights;
use crate::world::status::{ANIM_AWAY, ANIM_DO_NOT_DISTURB};
use egui::Color32;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Online,
    Away,
    Busy,
    Offline,
    Unknown,
}

/// What is known of a resident, gathered from the world.
#[derive(Debug, Clone, Copy, Default)]
pub struct Known {
    /// In the coarse locations of a region we are connected to.
    pub nearby: bool,
    /// Playing the away / do not disturb animation.
    pub away: bool,
    pub busy: bool,
    /// A friend who granted us their online status: whether they are online.
    pub friend_online: Option<bool>,
    /// Their last word was an automatic response.
    pub auto_responded: bool,
}

pub fn decide(k: Known) -> Status {
    if k.nearby {
        return animation_status(k.away, k.busy);
    }
    match k.friend_online {
        Some(true) => Status::Online,
        Some(false) => Status::Offline,
        None if k.auto_responded => Status::Busy,
        None => Status::Unknown,
    }
}

/// The status an avatar's animations give (do not disturb before away, as
/// on the tags).
pub fn animation_status(away: bool, busy: bool) -> Status {
    if busy {
        Status::Busy
    } else if away {
        Status::Away
    } else {
        Status::Online
    }
}

/// The status of an avatar in the scene, from its animations: the name
/// plates' source.
pub fn of_avatar(world: &World, id: &Uuid) -> Status {
    let anims = world.animations_of(id);
    let playing = |anim: Uuid| anims.iter().any(|a| a.id == anim);
    animation_status(playing(ANIM_AWAY), playing(ANIM_DO_NOT_DISTURB))
}

pub fn known(world: &World, id: &Uuid) -> Known {
    // Under RLVa's @shownearby, being around tells nothing: who stands
    // around is what RlvActions::canShowNearbyAgents (rlvactions.cpp) hides
    // from the nearby list and the radar, and the Legacy Viewer's
    // conversations and contacts take a status from the friend list alone.
    let nearby = !world.rlva.has("shownearby") && world.coarse.values().any(|v| v.iter().any(|(a, _)| a == id));
    let (away, busy) = if nearby {
        match of_avatar(world, id) {
            Status::Busy => (false, true),
            Status::Away => (true, false),
            _ => (false, false),
        }
    } else {
        (false, false)
    };
    let friend_online = world
        .social
        .friends
        .iter()
        .find(|f| f.id == *id)
        .filter(|f| f.rights_has & rights::ONLINE_STATUS != 0)
        .map(|f| f.online);
    Known {
        nearby,
        away,
        busy,
        friend_online,
        auto_responded: world.social.auto_responded.contains(id),
    }
}

pub fn of(world: &World, id: &Uuid) -> Status {
    decide(known(world, id))
}

/// `of` for a row that names `id` in `scope`: the same status in every
/// place. A list of people known besides the world (`NameScope::Roster`)
/// tells of a friend RLVa's `@shownames` hides around us what it tells of
/// anyone, as the Legacy Viewer's lists do: FSFloaterContacts
/// (indra/newview/fsfloatercontacts.cpp) and the group panels have no
/// `@shownames` test, and their menus already say who stands around
/// (`context::avatar_roster_menu`). A row of the world (`NameScope::World`)
/// names nobody the restriction hides, so its status tells of no one.
pub fn of_in(world: &World, _scope: NameScope, id: &Uuid) -> Status {
    of(world, id)
}

/// The color of a status dot: the one place to change what the dots say.
pub fn color(p: &Palette, status: Status) -> Color32 {
    match status {
        Status::Online => p.success,
        Status::Away => p.amber,
        Status::Busy => p.danger,
        Status::Offline | Status::Unknown => p.muted_dim,
    }
}

impl Status {
    pub fn color(self, p: &Palette) -> Color32 {
        color(p, self)
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Online => aurora_i18n::tr!("ui-chat-status-online"),
            Status::Away => aurora_i18n::tr!("common-away"),
            Status::Busy => aurora_i18n::tr!("common-do-not-disturb"),
            Status::Offline => aurora_i18n::tr!("common-offline"),
            Status::Unknown => aurora_i18n::tr!("ui-chat-status-unknown"),
        }
    }

    /// The head of a conversation shows a pill for what differs from the
    /// usual (online) and is known.
    pub fn pill(self) -> bool {
        matches!(self, Status::Away | Status::Busy | Status::Offline)
    }
}

/// What tells a status apart without its colour (#418, colour blindness):
/// the shape of its dot (a full disc online, a crescent
/// moon away, a disc crossed by a bar in do not disturb, a hollow ring
/// offline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Disc,
    Crescent,
    Bar,
    Ring,
}

impl Shape {
    /// The disc cut out of a crescent: its offset (towards the top left)
    /// and its radius, as fractions of the mark's radius.
    pub const CUT_OFFSET: f32 = -0.42;
    pub const CUT_RADIUS: f32 = 0.72;
    /// The bar across a disc (width, height) and the line of a ring, as
    /// fractions of the radius.
    pub const BAR: (f32, f32) = (1.15, 0.38);
    pub const RING: f32 = 0.42;
}

impl Status {
    pub fn shape(self) -> Shape {
        match self {
            Status::Online => Shape::Disc,
            Status::Away => Shape::Crescent,
            Status::Busy => Shape::Bar,
            Status::Offline | Status::Unknown => Shape::Ring,
        }
    }
}

/// A status mark of radius `r`: `shape` in `color`, its cuts in `hole`
/// (the surface it lies on, opaque: a translucent hole lets the mark show
/// through and the crescent reads as a disc, #744).
pub fn paint_mark(painter: &egui::Painter, center: egui::Pos2, r: f32, shape: Shape, color: Color32, hole: Color32) {
    match shape {
        Shape::Disc => {
            painter.circle_filled(center, r, color);
        }
        Shape::Crescent => {
            painter.circle_filled(center, r, color);
            painter.circle_filled(center + egui::Vec2::splat(r * Shape::CUT_OFFSET), r * Shape::CUT_RADIUS, hole);
        }
        Shape::Bar => {
            painter.circle_filled(center, r, color);
            let bar = egui::Rect::from_center_size(center, egui::vec2(r * Shape::BAR.0, r * Shape::BAR.1));
            painter.rect_filled(bar, bar.height() / 2.0, hole);
        }
        Shape::Ring => {
            let w = r * Shape::RING;
            painter.circle_stroke(center, r - w / 2.0, egui::Stroke::new(w, color));
        }
    }
}

/// The dot on the bottom right edge of a round picture, ringed in `ring`
/// (the surface under the picture, opaque) so that it reads over any
/// picture.
pub fn paint_dot(painter: &egui::Painter, p: &Palette, picture: egui::Rect, status: Status, ring: Color32) {
    let r = (picture.width() * 0.16).clamp(3.0, 6.0);
    let c = picture.center() + egui::Vec2::splat(picture.width() * 0.5 * std::f32::consts::FRAC_1_SQRT_2);
    painter.circle_filled(c, r + 1.5, ring);
    paint_mark(painter, c, r, status.shape(), status.color(p), ring);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_then_the_friend_list_then_an_automatic_response() {
        // nobody we know anything of
        assert_eq!(decide(Known::default()), Status::Unknown);
        // 1. in our regions: online, or what their avatar plays
        let near = Known {
            nearby: true,
            ..Known::default()
        };
        assert_eq!(decide(near), Status::Online);
        assert_eq!(decide(Known { away: true, ..near }), Status::Away);
        assert_eq!(decide(Known { busy: true, ..near }), Status::Busy);
        assert_eq!(
            decide(Known {
                away: true,
                busy: true,
                ..near
            }),
            Status::Busy
        );
        // the scene wins over a friend list late to say they came online,
        // and over an automatic response
        assert_eq!(
            decide(Known {
                friend_online: Some(false),
                auto_responded: true,
                ..near
            }),
            Status::Online
        );
        // 2. a friend's online status, granted, wins over an automatic response
        let friend = |on| Known {
            friend_online: Some(on),
            auto_responded: true,
            ..Known::default()
        };
        assert_eq!(decide(friend(true)), Status::Online);
        assert_eq!(decide(friend(false)), Status::Offline);
        // 3. otherwise an automatic response reads as do not disturb
        assert_eq!(
            decide(Known {
                auto_responded: true,
                ..Known::default()
            }),
            Status::Busy
        );
        assert!(!Status::Online.pill() && !Status::Unknown.pill());
        assert!(Status::Busy.pill() && Status::Away.pill() && Status::Offline.pill());
    }

    #[test]
    fn plates_and_conversations_share_their_colors() {
        let p = crate::theme::Theme::default().palette();
        assert_eq!(color(&p, animation_status(false, false)), p.success);
        assert_eq!(color(&p, animation_status(true, false)), p.amber);
        assert_eq!(color(&p, animation_status(false, true)), p.danger);
        assert_eq!(color(&p, Status::Unknown), color(&p, Status::Offline));
    }

    /// FSFloaterContacts has no `@shownames` test: the row of a friend the
    /// restriction hides around us says what it says without it.
    #[test]
    fn a_roster_row_keeps_its_status_under_shownames() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let friend = Uuid::from_u128(7);
        world.social.friends.push(crate::world::social::Friend {
            id: friend,
            online: true,
            rights_given: 0,
            rights_has: 0,
        });
        // around us, by the region's coarse locations
        world.coarse.entry(1).or_default().push((friend, glam::Vec3::ONE));
        assert_eq!(of(&world, &friend), Status::Online);
        world.rlva.configure(true);
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Presence fixture".into(),
            source_id: Uuid::from_u128(4),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: "@shownames=n".into(),
        }));
        world.refresh_rlva_privacy();
        assert!(!world.rlva_shows_name(NameScope::World, friend));
        for scope in [NameScope::Roster, NameScope::World] {
            assert_eq!(of_in(&world, scope, &friend), Status::Online, "{scope:?}");
        }
    }

    /// #418: the shape tells the status without the colour —
    /// a disc online, a crescent away, a disc with a bar in do not
    /// disturb, a hollow ring offline (unknown reads as offline).
    #[test]
    fn every_status_has_its_own_shape() {
        let shapes = [Status::Online, Status::Away, Status::Busy, Status::Offline].map(Status::shape);
        assert_eq!(shapes, [Shape::Disc, Shape::Crescent, Shape::Bar, Shape::Ring]);
        assert_eq!(Status::Unknown.shape(), Status::Offline.shape());
        // the cuts stay inside the mark
        const { assert!(Shape::BAR.0 < 2.0 && Shape::BAR.1 < 1.0 && Shape::RING < 1.0) };
        const { assert!(Shape::CUT_RADIUS < 1.0 && Shape::CUT_OFFSET < 0.0) };
    }
    /// Being around is nearby information: under @shownearby the status
    /// comes from the friend list alone.
    #[test]
    fn being_around_tells_no_status_under_shownearby() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let stranger = Uuid::from_u128(8);
        let friend = Uuid::from_u128(9);
        world.social.friends.push(crate::world::social::Friend {
            id: friend,
            online: true,
            rights_given: 0,
            rights_has: rights::ONLINE_STATUS,
        });
        let region = world.coarse.entry(1).or_default();
        region.push((stranger, glam::Vec3::ONE));
        region.push((friend, glam::Vec3::ONE));
        assert_eq!(of(&world, &stranger), Status::Online);
        world.rlva.configure(true);
        restrict(&mut world, "@shownearby=n");
        assert!(!known(&world, &stranger).nearby);
        assert_eq!(of(&world, &stranger), Status::Unknown, "a stranger around is not told online");
        assert_eq!(of(&world, &friend), Status::Online, "a friend's granted status stays");
        restrict(&mut world, "@shownearby=y");
        assert_eq!(of(&world, &stranger), Status::Online);
    }

    fn restrict(world: &mut World, command: &str) {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Privacy fixture".into(),
            source_id: Uuid::from_u128(4),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: command.into(),
        }));
    }
}
