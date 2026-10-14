//! The status dot of a conversation's resident: what the viewer really
//! knows of them, nothing guessed.
//!
//! - A resident in one of our regions (in the coarse locations) is online;
//!   away and do not disturb are the animations their avatar plays
//!   (ANIM_AGENT_AWAY / ANIM_AGENT_DO_NOT_DISTURB), the same signal as the
//!   name tags (LLVOAvatar::idleUpdateNameTagText).
//! - Elsewhere, a friend who lets us see their online status is online or
//!   offline (LLAvatarTracker::isBuddyOnline); away and do not disturb are
//!   never sent for them.
//! - Anyone else is unknown. The automatic response of an IM
//!   (IM_DO_NOT_DISTURB_AUTO_RESPONSE) is not a sign of do not disturb:
//!   Firestorm sends it as well for its autorespond modes and for rejected
//!   teleport offers or friendship requests (send_rejecting_tp_offers_message).

use crate::theme::Palette;
use crate::world::World;
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
}

pub fn decide(k: Known) -> Status {
    if k.nearby {
        return if k.busy {
            Status::Busy
        } else if k.away {
            Status::Away
        } else {
            Status::Online
        };
    }
    match k.friend_online {
        Some(true) => Status::Online,
        Some(false) => Status::Offline,
        None => Status::Unknown,
    }
}

pub fn known(world: &World, id: &Uuid) -> Known {
    let nearby = world.coarse.values().any(|v| v.iter().any(|(a, _)| a == id));
    let anims = world.animations_of(id);
    let playing = |anim: Uuid| anims.iter().any(|a| a.id == anim);
    let friend_online = world
        .social
        .friends
        .iter()
        .find(|f| f.id == *id)
        .filter(|f| f.rights_has & rights::ONLINE_STATUS != 0)
        .map(|f| f.online);
    Known {
        nearby,
        away: nearby && playing(ANIM_AWAY),
        busy: nearby && playing(ANIM_DO_NOT_DISTURB),
        friend_online,
    }
}

pub fn of(world: &World, id: &Uuid) -> Status {
    decide(known(world, id))
}

impl Status {
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Status::Online => p.success,
            Status::Away => p.amber,
            Status::Busy => p.danger,
            Status::Offline | Status::Unknown => p.muted_dim,
        }
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

    /// The header shows a pill for what differs from the usual (online) and
    /// is really known.
    pub fn pill(self) -> bool {
        matches!(self, Status::Away | Status::Busy | Status::Offline)
    }
}

/// The dot at the bottom right of a picture, ringed in `ring` (the surface
/// under the picture) so that it reads over any picture.
pub fn paint_dot(painter: &egui::Painter, p: &Palette, picture: egui::Rect, status: Status, ring: Color32) {
    let r = (picture.width() * 0.16).clamp(3.0, 6.0);
    let c = picture.right_bottom() - egui::vec2(r * 0.6, r * 0.6);
    painter.circle_filled(c, r + 1.5, ring);
    painter.circle_filled(c, r, status.color(p));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_signals_make_a_status() {
        // nobody we know anything of
        assert_eq!(decide(Known::default()), Status::Unknown);
        // a friend's online status, granted
        let friend = |on| Known {
            friend_online: Some(on),
            ..Known::default()
        };
        assert_eq!(decide(friend(true)), Status::Online);
        assert_eq!(decide(friend(false)), Status::Offline);
        // in our regions: online, or what their avatar plays
        let near = Known {
            nearby: true,
            ..Known::default()
        };
        assert_eq!(decide(near), Status::Online);
        assert_eq!(decide(Known { away: true, ..near }), Status::Away);
        assert_eq!(decide(Known { busy: true, ..near }), Status::Busy);
        // do not disturb wins over away, as on the name tags
        assert_eq!(
            decide(Known {
                away: true,
                busy: true,
                ..near
            }),
            Status::Busy
        );
        // nearby wins over a friend list that is late to say they came online
        assert_eq!(
            decide(Known {
                friend_online: Some(false),
                ..near
            }),
            Status::Online
        );
        assert!(!Status::Online.pill() && !Status::Unknown.pill());
        assert!(Status::Busy.pill() && Status::Away.pill() && Status::Offline.pill());
    }
}
