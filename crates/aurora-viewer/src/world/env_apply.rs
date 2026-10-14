//! Applying edited environment settings to the parcel or the region: the
//! editors' "Apply To Parcel / Region" and the day cycle editor's commit
//! (LLFloaterEditEnvironmentBase::doApplyEnvironment, LLFloaterFixedEnvironment::
//! onButtonApply, LLFloaterEditExtDayCycle::onButtonApply / doApplyCommit,
//! newview/llfloatereditenvironmentbase.cpp, llfloaterfixedenvironment.cpp,
//! llfloatereditextdaycycle.cpp; LLEnvironment::createDayCycleFromEnvironment
//! and canAgentUpdateParcelEnvironment, newview/llenvironment.cpp; originally
//! LGPL 2.1).
//!
//! An unchanged inventory item goes as its asset; changed settings go inline,
//! a sky or water wrapped in the current day of the parcel (or region) with
//! its track replaced. Settings using a local texture are never sent.

use super::env_day_edit::{DayDoc, TRACK_GROUND, TRACK_WATER, TRACKS};
use super::env_edit::{FLAG_NOMOD, FLAG_NOTRANS, SettingsDoc};
use super::env_select::SettingsKind;
use super::land::{AgentRights, modifiable_by_agent, powers};
use aurora_llsd::Llsd;
use aurora_net::ParcelInfo;
use uuid::Uuid;

/// Where settings are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A parcel of the agent's region, by its local id.
    Parcel(i32),
    Region,
}

impl Target {
    /// The ExtEnvironment parcel id (INVALID_PARCEL_ID for the region).
    pub fn local_id(self) -> i32 {
        match self {
            Target::Parcel(id) => id,
            Target::Region => aurora_net::land::REGION_PARCEL_ID,
        }
    }
}

/// LLEnvironment::canAgentUpdateParcelEnvironment: an EEP region, a parcel
/// whose estate allows its own environment, modifiable by the agent with the
/// group's "environment" power (we are never godlike).
pub fn can_update_parcel(parcel: &ParcelInfo, rights: &AgentRights, extended: bool) -> bool {
    extended && parcel.region_allow_env_override && modifiable_by_agent(parcel, rights, powers::LAND_ALLOW_ENVIRONMENT)
}

/// doApplyEnvironment's flags: no-modify / no-transfer of the item (none
/// without an item), with the settings' own flags.
pub fn apply_flags(item_owner_mask: Option<u32>, settings_flags: u32) -> u32 {
    let mut flags = settings_flags;
    if let Some(mask) = item_owner_mask {
        if mask & 0x4000 == 0 {
            flags |= FLAG_NOMOD;
        }
        if mask & 0x2000 == 0 {
            flags |= FLAG_NOTRANS;
        }
    }
    flags
}

/// LLEnvironment::createDayCycleFromEnvironment for a sky or water: the
/// current day (the parcel's, else the region's) with every sky track
/// cleared and the sky at the start of the ground track, or the water track
/// replaced by the water; the day takes the settings' flags
/// (updateParcel(…, psky / pwater): pday->setFlag(getFlags())).
pub fn day_around(current: &Llsd, frame: &SettingsDoc) -> Option<DayDoc> {
    let mut current = current.clone();
    if !current.has("type") {
        current.insert("type", "daycycle");
    }
    let mut day = DayDoc::from_llsd(current)?;
    let mut map = frame.llsd_ref().clone();
    if let Llsd::Map(m) = &mut map {
        m.remove("name");
    }
    match frame.kind() {
        SettingsKind::Sky => {
            for track in &mut day.tracks[TRACK_GROUND..TRACKS] {
                track.clear();
            }
            day.tracks[TRACK_GROUND].push((0.0, map));
        }
        SettingsKind::Water => day.tracks[TRACK_WATER] = vec![(0.0, map)],
        SettingsKind::Day => return None,
    }
    day.set_flags(day.flags() | frame.flags());
    Some(day)
}

/// A texture field of the settings (LLTrans "Environment*" strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalField {
    Sun,
    Moon,
    CloudNoise,
    Bloom,
    NormalMap,
    Transparent,
}

impl LocalField {
    pub fn label(self) -> &'static str {
        match self {
            LocalField::Sun => aurora_i18n::tr!("ui-environment-sun"),
            LocalField::Moon => aurora_i18n::tr!("ui-environment-moon"),
            LocalField::CloudNoise => aurora_i18n::tr!("ui-env-apply-cloud-noise"),
            LocalField::Bloom => aurora_i18n::tr!("ui-env-apply-bloom"),
            LocalField::NormalMap => aurora_i18n::tr!("ui-env-editor-normal-map"),
            LocalField::Transparent => aurora_i18n::tr!("ui-env-apply-transparent"),
        }
    }
}

/// The first texture of a sky or water using a local file, in
/// onButtonApply's order (water: normal map, transparent texture; sky: sun,
/// moon, cloud noise, bloom).
pub fn local_texture(map: &Llsd, water: bool, is_local: &dyn Fn(Uuid) -> bool) -> Option<LocalField> {
    let fields: &[(&str, LocalField)] = if water {
        &[
            ("normal_map", LocalField::NormalMap),
            ("transparent_texture", LocalField::Transparent),
        ]
    } else {
        &[
            ("sun_id", LocalField::Sun),
            ("moon_id", LocalField::Moon),
            ("cloud_id", LocalField::CloudNoise),
            ("bloom_id", LocalField::Bloom),
        ]
    };
    fields
        .iter()
        .find(|(key, _)| map.has(key) && is_local(map[*key].as_uuid()))
        .map(|(_, field)| *field)
}

/// A key frame of a day using a local texture (the day editor's
/// "brute-force local texture scan"): the track, the frame's number on its
/// track (from 1), its position and the field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalInDay {
    pub track: usize,
    pub frame: usize,
    pub pos: f32,
    pub field: LocalField,
}

pub fn local_texture_in_day(day: &DayDoc, is_local: &dyn Fn(Uuid) -> bool) -> Option<LocalInDay> {
    day.tracks.iter().enumerate().find_map(|(track, keys)| {
        keys.iter().enumerate().find_map(|(i, (pos, frame))| {
            local_texture(frame, track == TRACK_WATER, is_local).map(|field| LocalInDay {
                track,
                frame: i + 1,
                pos: *pos,
                field,
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::eep::{SkyFrame, WaterFrame};
    use aurora_llsd::llsd_map;

    fn current() -> Llsd {
        let sky = |g: f64| llsd_map! {"type" => "sky", "gamma" => g};
        llsd_map! {
            "name" => "Region day",
            "flags" => FLAG_NOTRANS as i32,
            "frames" => llsd_map! {
                "a" => sky(1.0),
                "b" => sky(2.0),
                "w" => llsd_map! {"type" => "water", "water_fog_density" => 4.0},
            },
            "tracks" => Llsd::Array(vec![
                Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "w"}]),
                Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "a"}, llsd_map! {"key_keyframe" => 0.5, "key_name" => "b"}]),
                Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.5, "key_name" => "b"}]),
            ]),
        }
    }

    #[test]
    fn a_sky_replaces_every_sky_track_of_the_current_day() {
        let mut sky = SettingsDoc::snapshot_sky(
            &SkyFrame {
                gamma: 3.0,
                ..SkyFrame::default()
            },
            "Mine",
        );
        sky.set_flags(FLAG_NOMOD);
        let day = day_around(&current(), &sky).expect("a day");
        assert_eq!(day.tracks[TRACK_GROUND].len(), 1);
        assert_eq!(day.tracks[TRACK_GROUND][0].0, 0.0);
        assert!(day.tracks[2].is_empty(), "the higher skies are cleared");
        assert_eq!(day.tracks[TRACK_WATER].len(), 1, "the water stays");
        assert!(!day.tracks[TRACK_GROUND][0].1.has("name"));
        assert_eq!(day.name(), "Region day");
        assert_eq!(day.flags(), FLAG_NOTRANS | FLAG_NOMOD);
        let rendered = day.day();
        assert!((rendered.sky_at(0.7, 0).gamma - 3.0).abs() < 1e-6);
        assert!((rendered.water_at(0.2).fog_density - 4.0).abs() < 1e-6);
    }

    #[test]
    fn a_water_replaces_the_water_track_only() {
        let water = SettingsDoc::snapshot_water(
            &WaterFrame {
                fog_density: 9.0,
                ..WaterFrame::default()
            },
            "W",
        );
        let day = day_around(&current(), &water).expect("a day");
        assert_eq!(day.tracks[TRACK_GROUND].len(), 2);
        assert_eq!(day.tracks[2].len(), 1);
        assert!((day.day().water_at(0.9).fog_density - 9.0).abs() < 1e-6);
    }

    #[test]
    fn flags_take_the_item_permissions() {
        assert_eq!(apply_flags(None, 0), 0);
        assert_eq!(apply_flags(Some(0x7fffffff), 0), 0);
        assert_eq!(apply_flags(Some(0x8000), 0), FLAG_NOMOD | FLAG_NOTRANS);
        assert_eq!(apply_flags(Some(0x4000 | 0x8000), FLAG_NOMOD), FLAG_NOMOD | FLAG_NOTRANS);
        assert_eq!(Target::Region.local_id(), -1);
        assert_eq!(Target::Parcel(3).local_id(), 3);
    }

    #[test]
    fn local_textures_are_found_in_firestorm_order() {
        let local = Uuid::from_u128(77);
        let is_local = |id: Uuid| id == local;
        let sky = llsd_map! {"type" => "sky", "moon_id" => local, "cloud_id" => local};
        assert_eq!(local_texture(&sky, false, &is_local), Some(LocalField::Moon));
        let water = llsd_map! {"type" => "water", "transparent_texture" => local};
        assert_eq!(local_texture(&water, true, &is_local), Some(LocalField::Transparent));
        assert_eq!(local_texture(&llsd_map! {"type" => "sky"}, false, &is_local), None);
        let mut day = DayDoc::from_llsd({
            let mut c = current();
            c.insert("type", "daycycle");
            c
        })
        .expect("a day");
        assert_eq!(local_texture_in_day(&day, &is_local), None);
        day.tracks[TRACK_GROUND][1].1.insert("sun_id", local);
        assert_eq!(
            local_texture_in_day(&day, &is_local),
            Some(LocalInDay {
                track: TRACK_GROUND,
                frame: 2,
                pos: 0.5,
                field: LocalField::Sun
            })
        );
    }
}
