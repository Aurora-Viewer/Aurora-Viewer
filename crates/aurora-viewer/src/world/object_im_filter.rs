//! "Block object messages from elsewhere" (Preferences › Privacy, issue
//! #623): the instant messages of objects (llInstantMessage, IM_FROM_TASK)
//! sent from another region, or from outside our parcel, are dropped, and so
//! is what an object outside our parcel says to us alone (llRegionSayTo).
//! Our own objects always pass, friends' objects unless unticked.
//!
//! The Legacy Viewer has no such switch: its rule-based filter
//! (omnifilterengine.h) can only name regions one by one. Where the message
//! comes from is the location that llimprocessing.cpp turns into the
//! object's link, "Region/x/y/z" in the binary bucket (RLVa cuts the region
//! name at the first '/'). A message whose origin is unknown passes:
//! system messages, regions that send no location, a parcel not known yet.

use super::World;
use super::landmarks::in_parcel;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage};
use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// IM_FROM_TASK.
const DIALOG_FROM_TASK: u8 = 19;
/// CHAT_TYPE_DIRECT: llRegionSayTo.
const CHAT_TYPE_DIRECT: u8 = 9;

/// How far an object may be and still reach us.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Scope {
    /// Objects in our region.
    #[default]
    Region,
    /// Objects on our parcel.
    Parcel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Off by default: shops, rentals and group servers write from afar.
    pub on: bool,
    pub scope: Scope,
    /// Friends' objects pass wherever they are.
    pub allow_friends: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: false,
            scope: Scope::Region,
            allow_friends: true,
        }
    }
}

/// Where an object's message was sent from.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub region: String,
    /// Region coordinates, when the location carries them.
    pub pos: Option<Vec3>,
}

/// The location of an IM_FROM_TASK binary bucket, "Region Name/x/y/z" ended
/// by a NUL (ll_safe_string(binary_bucket, binary_bucket_size - 1)). None
/// when there is none (system messages, older regions).
pub fn origin(bucket: &[u8]) -> Option<Origin> {
    let end = bucket.iter().position(|b| *b == 0).unwrap_or(bucket.len());
    let text = String::from_utf8_lossy(&bucket[..end]);
    let (region, coords) = text.split_once('/').unwrap_or((&text[..], ""));
    let region = unescape(region.trim());
    if region.is_empty() {
        return None;
    }
    let xyz: Vec<f32> = coords.split('/').map_while(|c| c.trim().parse().ok()).collect();
    let pos = (xyz.len() >= 3).then(|| Vec3::new(xyz[0], xyz[1], xyz[2]));
    Some(Origin { region, pos })
}

/// A region name written as a SLURL path ("Bay%20City"): region names hold
/// no '%', so decoding never changes a plain one.
fn unescape(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.filter(|_| b[i] == b'%').and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where we are, as far as it is known.
pub struct Here<'a> {
    pub agent: Uuid,
    /// Our region's name (empty before the region is known).
    pub region: &'a str,
    /// Is a region position on our parcel? None while the parcel is unknown.
    pub on_parcel: &'a dyn Fn(Vec3) -> Option<bool>,
}

/// The sender of an object's message.
pub struct Sender {
    pub owner: Uuid,
    /// A group-owned object: `owner` is the group.
    pub group_owned: bool,
    pub friend: bool,
}

/// Is this object's message dropped? `region` is None when the message
/// comes from our region (local chat), `pos` its position when known.
pub fn drops(cfg: &Settings, here: &Here, sender: &Sender, region: Option<&str>, pos: Option<Vec3>) -> bool {
    if !cfg.on || (!sender.group_owned && sender.owner == here.agent) {
        return false;
    }
    if cfg.allow_friends && !sender.group_owned && sender.friend {
        return false;
    }
    if let Some(region) = region
        && !here.region.is_empty()
        && !region.eq_ignore_ascii_case(here.region.trim())
    {
        return true;
    }
    cfg.scope == Scope::Parcel && pos.and_then(here.on_parcel) == Some(false)
}

impl World {
    /// Is a region position on our parcel (LLViewerParcelMgr::inAgentParcel)?
    fn on_agent_parcel(&self, pos: Vec3) -> Option<bool> {
        let (parcel, origin) = self.parcel.as_ref().zip(self.main_origin())?;
        if parcel.bitmap.is_empty() {
            return None;
        }
        let global = DVec3::new(f64::from(origin.0), f64::from(origin.1), 0.0) + pos.as_dvec3();
        Some(in_parcel(&parcel.bitmap, origin, global))
    }

    fn object_sender(&self, owner: Uuid, group_owned: bool) -> Sender {
        Sender {
            owner,
            group_owned,
            friend: self.social.is_friend(&owner),
        }
    }

    /// An object's IM from elsewhere, dropped.
    pub(super) fn object_im_filtered(&self, im: &InstantMessage) -> bool {
        if !self.object_im_filter.on || im.dialog != DIALOG_FROM_TASK {
            return false;
        }
        let Some(from) = origin(&im.binary_bucket) else {
            return false;
        };
        let region = self.region_name();
        let on_parcel = |p: Vec3| self.on_agent_parcel(p);
        let here = Here {
            agent: self.agent_id,
            region: &region,
            on_parcel: &on_parcel,
        };
        let sender = self.object_sender(im.from_agent_id, im.from_group);
        let dropped = drops(&self.object_im_filter, &here, &sender, Some(&from.region), from.pos);
        if dropped {
            log::info!("object message from elsewhere dropped: {} in {}", im.from_name, from.region);
        }
        dropped
    }

    /// What an object outside our parcel says to us alone (llRegionSayTo),
    /// dropped with the parcel scope. The owner of a ChatFromSimulator is
    /// never a group: a group-owned object's owner id is the group's, which
    /// is no friend nor us.
    pub(super) fn object_chat_filtered(&self, c: &ChatMessage) -> bool {
        if !self.object_im_filter.on || c.source_type != ChatSourceType::Object || c.chat_type != ChatType::Other(CHAT_TYPE_DIRECT) {
            return false;
        }
        let region = self.region_name();
        let on_parcel = |p: Vec3| self.on_agent_parcel(p);
        let here = Here {
            agent: self.agent_id,
            region: &region,
            on_parcel: &on_parcel,
        };
        let sender = self.object_sender(c.owner_id, false);
        let dropped = drops(&self.object_im_filter, &here, &sender, None, Some(c.position));
        if dropped {
            log::info!("object message from outside the parcel dropped: {}", c.from_name);
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    const ME: u128 = 1;
    const FRIEND: u128 = 2;
    const STRANGER: u128 = 3;

    fn on(scope: Scope) -> Settings {
        Settings {
            on: true,
            scope,
            allow_friends: true,
        }
    }

    /// Our parcel: x below 64 m.
    fn west_half(p: Vec3) -> Option<bool> {
        Some(p.x < 64.0)
    }

    fn unknown(_: Vec3) -> Option<bool> {
        None
    }

    fn here(on_parcel: &dyn Fn(Vec3) -> Option<bool>) -> Here<'_> {
        Here {
            agent: id(ME),
            region: "Bay City",
            on_parcel,
        }
    }

    fn sender(owner: u128) -> Sender {
        Sender {
            owner: id(owner),
            group_owned: false,
            friend: owner == FRIEND,
        }
    }

    const NEAR: Option<Vec3> = Some(Vec3::new(10.0, 10.0, 20.0));
    const FAR: Option<Vec3> = Some(Vec3::new(200.0, 10.0, 20.0));

    #[test]
    fn reads_the_location_of_the_bucket() {
        let o = origin(b"Bay City/128/64.5/22\0").unwrap();
        assert_eq!(o.region, "Bay City");
        assert_eq!(o.pos, Some(Vec3::new(128.0, 64.5, 22.0)));
        assert_eq!(origin(b"Bay%20City/1/2/3\0").unwrap().region, "Bay City");
        assert_eq!(origin(b"Bay City\0").unwrap().pos, None, "no coordinates");
        assert_eq!(origin(b"Bay City/1/x/3").unwrap().pos, None, "broken coordinates");
        assert_eq!(origin(b"\0"), None, "system message");
        assert_eq!(origin(b""), None);
        assert_eq!(origin(b"/1/2/3\0"), None, "no region name");
    }

    #[test]
    fn off_lets_everything_through() {
        let h = here(&west_half);
        let cfg = Settings::default();
        assert!(!cfg.on);
        assert!(!drops(&cfg, &h, &sender(STRANGER), Some("Elsewhere"), FAR));
    }

    #[test]
    fn region_scope_drops_other_regions_only() {
        let h = here(&west_half);
        let cfg = on(Scope::Region);
        assert!(drops(&cfg, &h, &sender(STRANGER), Some("Elsewhere"), NEAR));
        assert!(
            !drops(&cfg, &h, &sender(STRANGER), Some("Bay City"), FAR),
            "same region, other parcel"
        );
        assert!(!drops(&cfg, &h, &sender(STRANGER), Some("bay city"), NEAR), "case of the name");
        assert!(
            !drops(&cfg, &h, &sender(STRANGER), None, FAR),
            "llRegionSayTo comes from our region"
        );
    }

    #[test]
    fn parcel_scope_also_drops_the_rest_of_the_region() {
        let h = here(&west_half);
        let cfg = on(Scope::Parcel);
        assert!(drops(&cfg, &h, &sender(STRANGER), Some("Elsewhere"), NEAR));
        assert!(drops(&cfg, &h, &sender(STRANGER), Some("Bay City"), FAR));
        assert!(!drops(&cfg, &h, &sender(STRANGER), Some("Bay City"), NEAR));
        assert!(drops(&cfg, &h, &sender(STRANGER), None, FAR), "llRegionSayTo from another parcel");
        assert!(!drops(&cfg, &h, &sender(STRANGER), None, NEAR));
        assert!(!drops(&cfg, &h, &sender(STRANGER), Some("Bay City"), None), "position unknown");
        let h = here(&unknown);
        assert!(!drops(&cfg, &h, &sender(STRANGER), Some("Bay City"), FAR), "parcel unknown");
        assert!(
            drops(&cfg, &h, &sender(STRANGER), Some("Elsewhere"), FAR),
            "the region is still known"
        );
    }

    #[test]
    fn our_objects_always_pass_friends_unless_unticked() {
        let h = here(&west_half);
        let mut cfg = on(Scope::Parcel);
        cfg.allow_friends = false;
        assert!(!drops(&cfg, &h, &sender(ME), Some("Elsewhere"), FAR));
        assert!(drops(&cfg, &h, &sender(FRIEND), Some("Elsewhere"), FAR));
        cfg.allow_friends = true;
        assert!(!drops(&cfg, &h, &sender(FRIEND), Some("Elsewhere"), FAR));
        // a group's object: its owner id is the group's, never a friend nor us
        let group = Sender {
            owner: id(FRIEND),
            group_owned: true,
            friend: true,
        };
        assert!(drops(&cfg, &h, &group, Some("Elsewhere"), FAR));
    }

    #[test]
    fn an_unknown_region_of_ours_blocks_nothing_by_name() {
        let h = Here {
            agent: id(ME),
            region: "",
            on_parcel: &unknown,
        };
        assert!(!drops(&on(Scope::Parcel), &h, &sender(STRANGER), Some("Elsewhere"), FAR));
    }
}
