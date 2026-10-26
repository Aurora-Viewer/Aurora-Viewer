//! Freezing and ejecting a resident from our land: who may (the resident
//! menus show "Freeze" and "Eject" only then), whether the eject may also
//! ban them, and where they stand for the message.
//!
//! Port of LLAvatarActions::canLandFreezeOrEjectMultiple and the parcel
//! part of landEjectMultiple (indra/newview/llavataractions.cpp,
//! originally LGPL 2.1).

use super::World;
use super::land::{AgentRights, owned_by_agent, powers};
use super::landmarks::in_parcel;
use aurora_net::{ParcelInfo, RegionHandle};
use glam::{DVec3, Vec3};
use uuid::Uuid;

/// The ownership of a ParcelOverlay cell (its low three bits, PARCEL_* of
/// llparcel.h, read by LLViewerParcelOverlay::isOwnedSelf / isOwnedGroup).
const OWNERSHIP_MASK: u8 = 0x07;
const PARCEL_GROUP: u8 = 2;
const PARCEL_SELF: u8 = 3;
/// PARCEL_GRID_STEP_METERS.
const CELL: f32 = 4.0;

/// What we may do to a resident where they stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rights {
    /// "Freeze" and "Eject" (Avatar.EnableFreezeEject).
    pub allowed: bool,
    /// "Eject and Ban": the parcel under them is ours with the ban list
    /// (isParcelOwnedByAgent with GP_LAND_MANAGE_BANNED).
    pub ban: bool,
}

/// What the rule reads of the place a resident stands in.
pub struct Place<'a> {
    /// RegionHandshake SimOwner and IsEstateManager of their region.
    pub region_owner: Uuid,
    pub estate_manager: bool,
    /// The ParcelOverlay cell under them, once the region sent it.
    pub overlay: Option<u8>,
    /// The parcel under them, when the viewer knows it.
    pub parcel: Option<&'a ParcelInfo>,
}

/// canLandFreezeOrEjectMultiple for one resident (we are never godlike):
/// the region's owner, an estate manager, the owner of the parcel under
/// them, or a member of the group owning it with "Eject and freeze".
pub fn rights(place: &Place, agent: &AgentRights) -> Rights {
    let ownership = place.overlay.map(|cell| cell & OWNERSHIP_MASK);
    let group_admin = ownership == Some(PARCEL_GROUP) && place.parcel.is_some_and(|p| agent.has_power(&p.owner_id, powers::LAND_ADMIN));
    let allowed = place.region_owner == agent.id || place.estate_manager || ownership == Some(PARCEL_SELF) || group_admin;
    Rights {
        allowed,
        ban: allowed && place.parcel.is_some_and(|p| owned_by_agent(p, agent, powers::LAND_MANAGE_BANNED)),
    }
}

/// The parcel under a resident is group land the viewer does not know yet:
/// the group's powers decide, so it must be asked for.
pub fn needs_parcel(place: &Place) -> bool {
    place.parcel.is_none() && place.overlay.map(|cell| cell & OWNERSHIP_MASK) == Some(PARCEL_GROUP)
}

impl World {
    /// getRegionAndPosGlobalFromAgentID: the region a resident stands in
    /// and their position in it, from their avatar (seated ones through
    /// their seat) or, out of view, their place on the map.
    pub fn resident_place(&self, id: Uuid) -> Option<(RegionHandle, Vec3)> {
        let p = crate::autopilot::avatar_position(self, id).or_else(|| self.avatar_map_position(id))?;
        let (mx, my) = self.main_origin()?;
        let (gx, gy) = (f64::from(mx) + f64::from(p.x), f64::from(my) + f64::from(p.y));
        self.regions.iter().find_map(|(h, r)| {
            let (ox, oy) = aurora_net::handle_to_origin(*h);
            let (lx, ly) = (gx - f64::from(ox), gy - f64::from(oy));
            let (sx, sy) = (f64::from(r.heightmap.size_x.max(256)), f64::from(r.heightmap.size_y.max(256)));
            ((0.0..sx).contains(&lx) && (0.0..sy).contains(&ly)).then(|| (*h, Vec3::new(lx as f32, ly as f32, p.z)))
        })
    }

    /// The place `local` of region `handle` as the rule reads it; `known`
    /// is a parcel the caller holds besides ours and About Land's.
    pub fn moderation_place<'a>(&'a self, handle: RegionHandle, local: Vec3, known: Option<&'a ParcelInfo>) -> Option<Place<'a>> {
        let info = self.regions.get(&handle)?.info.as_ref()?;
        let overlay = self.map.overlays.get(&handle).and_then(|(cx, cy, data)| {
            let (i, j) = ((local.x / CELL).max(0.0) as u32, (local.y / CELL).max(0.0) as u32);
            (i < *cx && j < *cy).then(|| data.get((j * cx + i) as usize).copied()).flatten()
        });
        let origin = aurora_net::handle_to_origin(handle);
        let global = DVec3::new(
            f64::from(origin.0) + f64::from(local.x),
            f64::from(origin.1) + f64::from(local.y),
            0.0,
        );
        let ours = self.parcel.as_deref().filter(|_| self.main_region == Some(handle));
        let selected = self
            .land
            .sel
            .as_ref()
            .filter(|s| s.handle == handle)
            .and_then(|s| s.parcel.as_deref());
        let parcel = [ours, selected, known]
            .into_iter()
            .flatten()
            .find(|p| in_parcel(&p.bitmap, origin, global));
        Some(Place {
            region_owner: info.owner,
            estate_manager: info.is_estate_manager,
            overlay,
            parcel,
        })
    }

    /// Who we are, for the rule.
    pub fn moderation_agent(&self) -> AgentRights<'_> {
        AgentRights {
            id: self.agent_id,
            groups: &self.groups.groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::GroupMembership;

    const ME: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);
    const GROUP: Uuid = Uuid::from_u128(3);

    fn place(overlay: u8, parcel: Option<&ParcelInfo>) -> Place<'_> {
        Place {
            region_owner: OTHER,
            estate_manager: false,
            overlay: Some(overlay),
            parcel,
        }
    }

    fn parcel(owner: Uuid) -> ParcelInfo {
        ParcelInfo {
            owner_id: owner,
            is_group_owned: owner == GROUP,
            ..ParcelInfo::default()
        }
    }

    fn member(powers: u64) -> Vec<GroupMembership> {
        vec![GroupMembership {
            id: GROUP,
            name: "Land group".into(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }]
    }

    #[test]
    fn region_owner_and_estate_managers_moderate_any_land() {
        let agent = AgentRights { id: ME, groups: &[] };
        let mut p = place(1, None);
        assert!(!rights(&p, &agent).allowed, "someone else's parcel");
        p.region_owner = ME;
        assert!(rights(&p, &agent).allowed);
        p.region_owner = OTHER;
        p.estate_manager = true;
        assert_eq!(rights(&p, &agent), Rights { allowed: true, ban: false });
    }

    #[test]
    fn our_parcel_by_the_overlay_and_its_ban_list() {
        let agent = AgentRights { id: ME, groups: &[] };
        // PARCEL_SELF with the border bits of the overlay cell
        assert_eq!(rights(&place(3 | 0xC0, None), &agent), Rights { allowed: true, ban: false });
        let ours = parcel(ME);
        assert_eq!(rights(&place(3, Some(&ours)), &agent), Rights { allowed: true, ban: true });
        assert!(!rights(&place(0, None), &agent).allowed, "public land");
        assert!(!rights(&place(4, None), &agent).allowed, "for sale");
    }

    #[test]
    fn group_land_needs_the_eject_and_freeze_power() {
        let land = parcel(GROUP);
        let groups = member(powers::LAND_ADMIN);
        let agent = AgentRights { id: ME, groups: &groups };
        assert!(needs_parcel(&place(2, None)));
        assert!(!rights(&place(2, None), &agent).allowed, "the parcel is not known yet");
        assert!(!needs_parcel(&place(2, Some(&land))));
        assert_eq!(rights(&place(2, Some(&land)), &agent), Rights { allowed: true, ban: false });
        let groups = member(powers::LAND_ADMIN | powers::LAND_MANAGE_BANNED);
        let agent = AgentRights { id: ME, groups: &groups };
        assert_eq!(rights(&place(2, Some(&land)), &agent), Rights { allowed: true, ban: true });
        let groups = member(powers::LAND_MANAGE_BANNED);
        let agent = AgentRights { id: ME, groups: &groups };
        assert!(!rights(&place(2, Some(&land)), &agent).allowed, "banning alone is not ejecting");
    }

    #[test]
    fn no_overlay_yet_leaves_only_the_region_rights() {
        let agent = AgentRights { id: ME, groups: &[] };
        let p = Place {
            region_owner: OTHER,
            estate_manager: false,
            overlay: None,
            parcel: None,
        };
        assert!(!rights(&p, &agent).allowed);
        assert!(!needs_parcel(&p));
    }
}
