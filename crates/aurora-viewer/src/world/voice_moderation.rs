//! Nearby voice moderation: who may mute others in the spatial voice, and
//! the notice when a moderator mutes us.
//!
//! Port of LLNearbyVoiceModeration::isNearbyChatModerator and
//! showMutedNotification (indra/newview/llnearbyvoicemoderation.cpp), with
//! LLViewerParcelMgr::isVoiceRestricted / allowVoiceModeration /
//! isParcelOwnedByAgent (llviewerparcelmgr.cpp), originally LGPL 2.1. The
//! request itself is aurora_net::voice_moderation.

use super::{World, notifications};
use aurora_net::{ParcelInfo, parcel_flags};
use uuid::Uuid;

/// GP_SESSION_MODERATOR (roles_constants.h): can mute people's session.
const GP_SESSION_MODERATOR: u64 = 1 << 37;

/// isNearbyChatModerator once we are on the nearby channel: on a parcel
/// with its own voice (isVoiceRestricted) its owner, or a member of its
/// owning group with the moderator power; elsewhere an estate manager.
pub fn may_moderate(parcel: Option<&ParcelInfo>, agent: Uuid, group_powers: Option<u64>, manages_estate: bool) -> bool {
    let Some(parcel) = parcel else {
        return false;
    };
    if parcel.flags & parcel_flags::USE_ESTATE_VOICE_CHAN != 0 {
        return manages_estate;
    }
    // isParcelOwnedByAgent: public land is nobody's
    if parcel.owner_id.is_nil() {
        return false;
    }
    if parcel.is_group_owned {
        return group_powers.is_some_and(|p| p & GP_SESSION_MODERATOR != 0);
    }
    parcel.owner_id == agent
}

impl World {
    /// Whether a moderator muted `id` in nearby voice, when we may moderate
    /// it and they are on the channel; None otherwise (no entries).
    pub fn nearby_voice_moderated(&self, id: &Uuid) -> Option<bool> {
        let moderated = self.calls.nearby_moderated.as_ref()?.get(id).copied()?;
        let parcel = self.parcel.as_deref();
        let powers = parcel
            .filter(|p| p.is_group_owned)
            .and_then(|p| self.groups.group(&p.owner_id))
            .map(|g| g.powers);
        let manages_estate = self
            .main()
            .and_then(|r| r.info.as_ref())
            .is_some_and(|i| i.is_estate_manager || i.owner == self.agent_id);
        may_moderate(parcel, self.agent_id, powers, manages_estate).then_some(moderated)
    }

    /// NearbyVoiceMutedByModerator / NearbyVoiceUnmutedByModerator.
    pub fn on_nearby_voice_moderated(&mut self, muted: bool) {
        log::info!("voice: a moderator {} us in nearby voice", if muted { "muted" } else { "unmuted" });
        let text = if muted {
            aurora_i18n::tr!("world-mod-voice-muted-by-moderator")
        } else {
            aurora_i18n::tr!("world-mod-voice-unmuted-by-moderator")
        };
        self.notifications.push(
            notifications::Kind::Info,
            aurora_i18n::tr!("world-mod-voice-moderation"),
            text.to_owned(),
            notifications::Data::None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parcel(owner: u128, group_owned: bool, estate_voice: bool) -> ParcelInfo {
        ParcelInfo {
            owner_id: Uuid::from_u128(owner),
            is_group_owned: group_owned,
            flags: if estate_voice { parcel_flags::USE_ESTATE_VOICE_CHAN } else { 0 },
            ..Default::default()
        }
    }

    #[test]
    fn the_estate_channel_is_moderated_by_estate_managers() {
        let me = Uuid::from_u128(1);
        let p = parcel(2, false, true);
        assert!(may_moderate(Some(&p), me, None, true));
        assert!(!may_moderate(Some(&p), me, None, false));
        // even our own parcel, when it uses the estate channel
        assert!(!may_moderate(Some(&parcel(1, false, true)), me, None, false));
        assert!(!may_moderate(None, me, None, true));
    }

    #[test]
    fn a_parcel_channel_is_moderated_by_its_owner() {
        let me = Uuid::from_u128(1);
        assert!(may_moderate(Some(&parcel(1, false, false)), me, None, false));
        // an estate manager has no say on a parcel's own channel
        assert!(!may_moderate(Some(&parcel(2, false, false)), me, None, true));
        assert!(!may_moderate(Some(&parcel(0, false, false)), me, None, true));
        // a group's parcel: its moderators only
        let group = parcel(9, true, false);
        assert!(may_moderate(Some(&group), me, Some(GP_SESSION_MODERATOR), false));
        assert!(!may_moderate(Some(&group), me, Some(0), true));
        assert!(!may_moderate(Some(&group), me, None, true));
    }
}
