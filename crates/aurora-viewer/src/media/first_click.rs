//! Which media takes the first click, with no focus click first. Port of
//! LLToolPie::shouldAllowFirstMediaInteraction (indra/newview/lltoolpie.cpp)
//! and of the MediaFirstClickInteract bit field (MediaFirstClickTypes,
//! lltoolpie.h): one switch per kind, worn HUDs, own, friends', group and
//! landowner's objects on as its default (31).

use uuid::Uuid;

/// The kinds of MediaFirstClickInteract (Preferences › Sound and media).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FirstClick {
    /// MEDIA_FIRST_CLICK_HUD: our worn HUDs.
    pub hud: bool,
    /// MEDIA_FIRST_CLICK_OWN: objects we own.
    pub own: bool,
    /// MEDIA_FIRST_CLICK_FRIEND: objects a friend owns.
    pub friend: bool,
    /// MEDIA_FIRST_CLICK_GROUP: objects set to, or owned by, one of our
    /// groups.
    pub group: bool,
    /// MEDIA_FIRST_CLICK_LAND: objects of the parcel's owner or group, on
    /// the parcel we stand on.
    pub land: bool,
    /// MEDIA_FIRST_CLICK_ANY: anyone's object.
    pub any: bool,
    /// MEDIA_FIRST_CLICK_BYPASS_MOAP_FLAG: also the faces whose media does
    /// not ask for it (PRIM_MEDIA_FIRST_CLICK_INTERACT off).
    pub all_faces: bool,
}

impl Default for FirstClick {
    fn default() -> Self {
        FirstClick {
            hud: true,
            own: true,
            friend: true,
            group: true,
            land: true,
            any: false,
            all_faces: false,
        }
    }
}

/// What the viewer knows of the clicked object and of the agent.
pub struct Clicked<'a> {
    /// A worn HUD.
    pub hud: bool,
    /// Owner and group of the object (the hover node's permissions:
    /// ObjectPropertiesFamily); `None` before they have come.
    pub owner: Option<(Uuid, Uuid)>,
    pub agent: Uuid,
    pub is_friend: &'a dyn Fn(&Uuid) -> bool,
    pub in_group: &'a dyn Fn(&Uuid) -> bool,
    /// Owner and group of the agent's parcel, when the object stands on it
    /// (LLViewerParcelMgr::inAgentParcel).
    pub parcel: Option<(Uuid, Uuid)>,
}

/// LLToolPie::shouldAllowFirstMediaInteraction. `enabled` is the master
/// switch (MediaFirstClickInteract 0 when off); `face_asks` the face's
/// PRIM_MEDIA_FIRST_CLICK_INTERACT.
pub fn allowed(enabled: bool, kinds: &FirstClick, face_asks: bool, c: &Clicked) -> bool {
    if !enabled || (!face_asks && !kinds.all_faces) {
        return false;
    }
    if kinds.any || (kinds.hud && c.hud) {
        return true;
    }
    // "Owner information was not reliably obtained"
    let Some((owner, group)) = c.owner.filter(|(o, g)| !o.is_nil() || !g.is_nil()) else {
        return false;
    };
    if kinds.own && owner == c.agent {
        return true;
    }
    if kinds.friend && !owner.is_nil() && (c.is_friend)(&owner) {
        return true;
    }
    if kinds.group && ((!group.is_nil() && (c.in_group)(&group)) || (!owner.is_nil() && (c.in_group)(&owner))) {
        return true;
    }
    // the parcel's owner, or its group: stricter than the Legacy Viewer,
    // whose `group_id == parcel_group` also holds when both are unset
    kinds.land
        && c.parcel.is_some_and(|(parcel_owner, parcel_group)| {
            (!parcel_owner.is_nil() && owner == parcel_owner) || (!parcel_group.is_nil() && group == parcel_group)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Uuid = Uuid::from_u128(1);
    const FRIEND: Uuid = Uuid::from_u128(2);
    const STRANGER: Uuid = Uuid::from_u128(3);
    const MY_GROUP: Uuid = Uuid::from_u128(4);
    const OTHER_GROUP: Uuid = Uuid::from_u128(5);

    fn check(kinds: &FirstClick, face_asks: bool, hud: bool, owner: Option<(Uuid, Uuid)>, parcel: Option<(Uuid, Uuid)>) -> bool {
        let is_friend = |id: &Uuid| *id == FRIEND;
        let in_group = |id: &Uuid| *id == MY_GROUP;
        let c = Clicked {
            hud,
            owner,
            agent: ME,
            is_friend: &is_friend,
            in_group: &in_group,
            parcel,
        };
        allowed(true, kinds, face_asks, &c)
    }

    #[test]
    fn the_default_is_the_legacy_31_and_follows_the_face() {
        let k = FirstClick::default();
        let nil = Uuid::nil();
        assert!(check(&k, true, true, None, None), "a worn HUD needs no owner");
        assert!(check(&k, true, false, Some((ME, nil)), None));
        assert!(check(&k, true, false, Some((FRIEND, nil)), None));
        assert!(check(&k, true, false, Some((STRANGER, MY_GROUP)), None));
        assert!(check(&k, true, false, Some((MY_GROUP, nil)), None), "group-owned");
        assert!(!check(&k, true, false, Some((STRANGER, OTHER_GROUP)), None));
        assert!(!check(&k, true, false, None, None), "owner not known yet");
        // the face did not ask for it
        assert!(!check(&k, false, false, Some((ME, nil)), None));
        assert!(!check(&k, false, true, None, None));
        let all = FirstClick { all_faces: true, ..k };
        assert!(check(&all, false, false, Some((ME, nil)), None));
    }

    #[test]
    fn the_land_kind_needs_the_parcel_owner_or_group() {
        let k = FirstClick {
            hud: false,
            own: false,
            friend: false,
            group: false,
            ..FirstClick::default()
        };
        let nil = Uuid::nil();
        assert!(check(&k, true, false, Some((STRANGER, nil)), Some((STRANGER, nil))));
        assert!(check(&k, true, false, Some((STRANGER, OTHER_GROUP)), Some((nil, OTHER_GROUP))));
        assert!(!check(&k, true, false, Some((STRANGER, nil)), Some((FRIEND, nil))));
        // unset groups do not match (stricter than the Legacy Viewer)
        assert!(!check(&k, true, false, Some((STRANGER, nil)), Some((FRIEND, nil))));
        assert!(!check(&k, true, false, Some((STRANGER, nil)), Some((nil, nil))));
        // not on the agent's parcel
        assert!(!check(&k, true, false, Some((STRANGER, nil)), None));
    }

    #[test]
    fn any_takes_every_object_and_off_takes_none() {
        let any = FirstClick {
            any: true,
            ..FirstClick::default()
        };
        assert!(check(&any, true, false, None, None));
        let none = FirstClick {
            hud: false,
            own: false,
            friend: false,
            group: false,
            land: false,
            any: false,
            all_faces: false,
        };
        assert!(!check(&none, true, true, Some((ME, Uuid::nil())), None));
        let is_friend = |_: &Uuid| true;
        let c = Clicked {
            hud: true,
            owner: Some((ME, Uuid::nil())),
            agent: ME,
            is_friend: &is_friend,
            in_group: &is_friend,
            parcel: None,
        };
        assert!(!allowed(false, &any, true, &c), "the master switch off");
    }
}
