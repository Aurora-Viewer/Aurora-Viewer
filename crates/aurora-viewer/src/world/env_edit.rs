//! A sky or water settings document being edited (the `mSettings` of
//! LLFloaterFixedEnvironment, newview/llfloaterfixedenvironment.cpp).
//!
//! The settings map is kept as it was loaded and only the keys the editor
//! changes are written back, so what it does not show (Rayleigh / Mie /
//! absorption profiles, rainbow and halo textures, keys of newer viewers)
//! is saved untouched. The setters follow LLSettingsSky / LLSettingsWater
//! (llinventory/llsettingssky.cpp, llsettingswater.cpp): an edited legacy
//! haze value moves into `legacy_haze` (saveValuesToLLSD / set_legacy), a
//! probe ambiance makes the sky non-classic. Windlight XML presets are
//! converted like translateLegacySettings and buildFromLegacyPreset
//! (newview/llsettingsvo.cpp). All originally LGPL 2.1.

use super::eep::{self, Settings, SkyFrame, WaterFrame};
use super::env_select::SettingsKind;
use aurora_llsd::Llsd;
use glam::{Quat, Vec2, Vec3};

/// LLSettingsBase flags (the `flags` key).
pub const FLAG_NOCOPY: u32 = 1;
pub const FLAG_NOMOD: u32 = 1 << 1;
pub const FLAG_NOTRANS: u32 = 1 << 2;
pub const FLAG_NOSAVE: u32 = 1 << 3;

/// Haze keys that live in `legacy_haze` once edited.
const LEGACY_HAZE: [&str; 7] = [
    "ambient",
    "blue_density",
    "blue_horizon",
    "density_multiplier",
    "distance_multiplier",
    "haze_density",
    "haze_horizon",
];

/// Sky and water values the frames do not carry, with the defaults of
/// LLSettingsSky::defaults() / LLSettingsWater::defaults().
pub const MOISTURE_LEVEL: (&str, f32) = ("moisture_level", 0.0);
pub const DROPLET_RADIUS: (&str, f32) = ("droplet_radius", 800.0);
pub const ICE_LEVEL: (&str, f32) = ("ice_level", 0.0);
pub const SCALE_ABOVE: (&str, f32) = ("scale_above", 0.0299);
pub const SCALE_BELOW: (&str, f32) = ("scale_below", 0.2);

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsDoc {
    kind: SettingsKind,
    map: Llsd,
}

impl SettingsDoc {
    /// A sky or water settings map (a day cycle is the day editor's).
    pub fn from_llsd(map: Llsd) -> Option<SettingsDoc> {
        let kind = match (map.is_map(), map["type"].as_str()) {
            (true, "sky") => SettingsKind::Sky,
            (true, "water") => SettingsKind::Water,
            _ => return None,
        };
        Some(SettingsDoc { kind, map })
    }

    /// A downloaded settings asset.
    pub fn from_asset(data: &[u8]) -> Option<SettingsDoc> {
        Settings::decode_asset(data).and_then(SettingsDoc::from_llsd)
    }

    /// A clone of what is shown, as the editors start without an item
    /// (getEnvironmentFixedSky / Water(ENV_CURRENT)->buildClone()).
    pub fn snapshot_sky(sky: &SkyFrame, name: &str) -> SettingsDoc {
        let mut map = sky.settings_llsd();
        map.insert("name", name);
        SettingsDoc {
            kind: SettingsKind::Sky,
            map,
        }
    }

    pub fn snapshot_water(water: &WaterFrame, name: &str) -> SettingsDoc {
        let mut map = water.settings_llsd();
        map.insert(SCALE_ABOVE.0, SCALE_ABOVE.1);
        map.insert(SCALE_BELOW.0, SCALE_BELOW.1);
        map.insert("name", name);
        SettingsDoc {
            kind: SettingsKind::Water,
            map,
        }
    }

    pub fn kind(&self) -> SettingsKind {
        self.kind
    }

    /// The settings map (a day cycle's frame).
    pub fn llsd_ref(&self) -> &Llsd {
        &self.map
    }

    pub fn into_llsd(self) -> Llsd {
        self.map
    }

    pub fn name(&self) -> &str {
        self.map["name"].as_str()
    }

    pub fn set_name(&mut self, name: &str) {
        self.map.insert("name", name);
    }

    pub fn flags(&self) -> u32 {
        self.map["flags"].as_u32()
    }

    pub fn set_flags(&mut self, flags: u32) {
        self.map.insert("flags", flags as i32);
    }

    /// The sky as rendered (missing keys take the defaults).
    pub fn sky(&self) -> SkyFrame {
        match Settings::from_llsd(&self.map) {
            Some(Settings::Sky(s)) => s,
            _ => SkyFrame::default(),
        }
    }

    pub fn water(&self) -> WaterFrame {
        match Settings::from_llsd(&self.map) {
            Some(Settings::Water(w)) => w,
            _ => WaterFrame::default(),
        }
    }

    /// A value the frames do not carry (`(key, default)`).
    pub fn extra(&self, (key, default): (&str, f32)) -> f32 {
        let v = self.map[key].as_f32();
        if self.map.has(key) && v.is_finite() { v } else { default }
    }

    pub fn set_extra(&mut self, (key, _): (&str, f32), v: f32) {
        self.map.insert(key, v);
    }

    /// LLSettingsSky's legacy haze setters: the value goes into
    /// `legacy_haze` and leaves the outer map.
    fn set_haze(&mut self, key: &str, v: Llsd) {
        if let Llsd::Map(m) = &mut self.map {
            m.remove(key);
            let haze = m.entry("legacy_haze".to_owned()).or_insert_with(Llsd::new_map);
            if !haze.is_map() {
                *haze = Llsd::new_map();
            }
            haze.insert(key, v);
        }
    }

    fn set(&mut self, key: &str, v: Llsd) {
        if LEGACY_HAZE.contains(&key) {
            self.set_haze(key, v);
        } else {
            self.map.insert(key, v);
        }
    }

    /// Write the fields of `after` that differ from `before` (the panels'
    /// setters, one per control). Returns whether anything changed.
    pub fn edit_sky(&mut self, before: &SkyFrame, after: &SkyFrame) -> bool {
        let mut changed = false;
        let mut v3 = |doc: &mut Self, key: &str, a: Vec3, b: Vec3| {
            if a != b {
                doc.set(key, eep::llsd_vec3(b));
                changed = true;
            }
        };
        v3(self, "sunlight_color", before.sunlight, after.sunlight);
        v3(self, "ambient", before.ambient, after.ambient);
        v3(self, "blue_horizon", before.blue_horizon, after.blue_horizon);
        v3(self, "blue_density", before.blue_density, after.blue_density);
        v3(self, "glow", before.glow, after.glow);
        v3(self, "cloud_color", before.cloud_color, after.cloud_color);
        v3(self, "cloud_pos_density1", before.cloud_pos_density1, after.cloud_pos_density1);
        v3(self, "cloud_pos_density2", before.cloud_pos_density2, after.cloud_pos_density2);
        let mut real = |doc: &mut Self, key: &str, a: f32, b: f32| {
            if a != b {
                doc.set(key, Llsd::Real(b as f64));
                changed = true;
            }
        };
        real(self, "haze_density", before.haze_density, after.haze_density);
        real(self, "haze_horizon", before.haze_horizon, after.haze_horizon);
        real(self, "density_multiplier", before.density_multiplier, after.density_multiplier);
        real(self, "distance_multiplier", before.distance_multiplier, after.distance_multiplier);
        real(self, "max_y", before.max_y, after.max_y);
        real(self, "cloud_scale", before.cloud_scale, after.cloud_scale);
        real(self, "cloud_shadow", before.cloud_shadow, after.cloud_shadow);
        real(self, "cloud_variance", before.cloud_variance, after.cloud_variance);
        real(self, "sun_scale", before.sun_scale, after.sun_scale);
        real(self, "moon_scale", before.moon_scale, after.moon_scale);
        real(self, "moon_brightness", before.moon_brightness, after.moon_brightness);
        real(self, "star_brightness", before.star_brightness, after.star_brightness);
        real(self, "gamma", before.gamma, after.gamma);
        if before.cloud_scroll_rate != after.cloud_scroll_rate {
            self.set("cloud_scroll_rate", eep::llsd_vec2(after.cloud_scroll_rate));
            changed = true;
        }
        for (key, a, b) in [
            ("sun_rotation", before.sun_rotation, after.sun_rotation),
            ("moon_rotation", before.moon_rotation, after.moon_rotation),
        ] {
            if a != b {
                self.set(key, eep::llsd_quat(b));
                changed = true;
            }
        }
        for (key, a, b) in [
            ("cloud_id", before.cloud_id, after.cloud_id),
            ("sun_id", before.sun_id, after.sun_id),
            ("moon_id", before.moon_id, after.moon_id),
        ] {
            if a != b {
                self.set(key, Llsd::Uuid(b));
                changed = true;
            }
        }
        // setReflectionProbeAmbiance: the sky stops being classic
        if before.probe_ambiance != after.probe_ambiance || before.can_auto_adjust != after.can_auto_adjust {
            if after.can_auto_adjust {
                if let Llsd::Map(m) = &mut self.map {
                    m.remove("reflection_probe_ambiance");
                }
            } else {
                self.set("reflection_probe_ambiance", Llsd::Real(after.probe_ambiance as f64));
            }
            changed = true;
        }
        changed
    }

    pub fn edit_water(&mut self, before: &WaterFrame, after: &WaterFrame) -> bool {
        let mut changed = false;
        if before.fog_color != after.fog_color {
            self.set("water_fog_color", eep::llsd_vec3(after.fog_color));
            changed = true;
        }
        if before.normal_scale != after.normal_scale {
            self.set("normal_scale", eep::llsd_vec3(after.normal_scale));
            changed = true;
        }
        for (key, a, b) in [
            ("water_fog_density", before.fog_density, after.fog_density),
            ("underwater_fog_mod", before.underwater_fog_mod, after.underwater_fog_mod),
            ("fresnel_scale", before.fresnel_scale, after.fresnel_scale),
            ("fresnel_offset", before.fresnel_offset, after.fresnel_offset),
            ("blur_multiplier", before.blur_multiplier, after.blur_multiplier),
        ] {
            if a != b {
                self.set(key, Llsd::Real(b as f64));
                changed = true;
            }
        }
        for (key, a, b) in [
            ("wave1_direction", before.wave1, after.wave1),
            ("wave2_direction", before.wave2, after.wave2),
        ] {
            if a != b {
                self.set(key, eep::llsd_vec2(b));
                changed = true;
            }
        }
        if before.normal_map != after.normal_map {
            self.set("normal_map", Llsd::Uuid(after.normal_map));
            changed = true;
        }
        changed
    }

    /// The asset to upload: LLSDSerialize::serialize(..., LLSD_NOTATION),
    /// header included (LLSettingsVOBase::updateInventoryItem).
    pub fn to_asset(&self) -> Vec<u8> {
        format!("<? llsd/notation ?>\n{}", aurora_llsd::to_notation(&self.map)).into_bytes()
    }

    /// A Windlight sky preset file (LLSettingsVOSky::buildFromLegacyPreset):
    /// None when nothing could be converted ("WLImportFail").
    pub fn import_legacy_sky(xml: &[u8], name: &str) -> Option<SettingsDoc> {
        let legacy = aurora_llsd::from_xml(xml).ok()?;
        let mut map = translate_legacy_sky(&legacy)?;
        map.insert("name", name);
        SettingsDoc::from_llsd(map)
    }

    /// A Windlight water preset file (LLSettingsVOWater::buildFromLegacyPreset).
    pub fn import_legacy_water(xml: &[u8], name: &str) -> Option<SettingsDoc> {
        let legacy = aurora_llsd::from_xml(xml).ok()?;
        let mut map = translate_legacy_water(&legacy)?;
        map.insert("name", name);
        SettingsDoc::from_llsd(map)
    }
}

/// The preset name of a Windlight file: its name without `.xml`, unescaped
/// (LLURI::unescape of the file name, as read_legacy_preset_data names it).
pub fn legacy_preset_name(file_stem: &str) -> String {
    let bytes = file_stem.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn color3(v: &Llsd) -> Llsd {
    eep::llsd_vec3(Vec3::new(v.at(0).as_f32(), v.at(1).as_f32(), v.at(2).as_f32()))
}

/// First component of a Windlight scalar (stored as `[x, 0, 0, 1]`).
fn first(v: &Llsd) -> Llsd {
    Llsd::Real(if v.is_array() { v.at(0).as_f64() } else { v.as_f64() })
}

/// convert_azimuth_and_altitude_to_quat (llsettingssky.cpp).
fn azimuth_altitude_quat(azimuth: f32, altitude: f32) -> Quat {
    let dir = Vec3::new(azimuth.cos() * altitude.cos(), azimuth.sin() * altitude.cos(), altitude.sin());
    Quat::from_rotation_arc(Vec3::X, dir.normalize_or(Vec3::X))
}

/// LLSettingsSky::translateLegacySettings on LLSettingsSky::defaults().
fn translate_legacy_sky(legacy: &Llsd) -> Option<Llsd> {
    let mut out = SkyFrame::default().settings_llsd();
    out.insert(MOISTURE_LEVEL.0, MOISTURE_LEVEL.1);
    out.insert(DROPLET_RADIUS.0, DROPLET_RADIUS.1);
    out.insert(ICE_LEVEL.0, ICE_LEVEL.1);
    let mut converted = false;
    // translateLegacyHazeSettings
    let mut haze = Llsd::new_map();
    for key in ["ambient", "blue_density", "blue_horizon"] {
        if legacy.has(key) {
            haze.insert(key, color3(&legacy[key]));
        }
    }
    for key in ["density_multiplier", "distance_multiplier", "haze_density", "haze_horizon"] {
        if legacy.has(key) {
            haze.insert(key, Llsd::Real(legacy[key].at(0).as_f64()));
        }
    }
    if !haze.is_empty() {
        out.insert("legacy_haze", haze);
        converted = true;
    }
    for key in ["cloud_color", "cloud_pos_density1", "cloud_pos_density2", "glow"] {
        if legacy.has(key) {
            out.insert(key, color3(&legacy[key]));
            converted = true;
        }
    }
    for key in ["cloud_scale", "cloud_shadow", "gamma", "max_y"] {
        if legacy.has(key) {
            out.insert(key, first(&legacy[key]));
            converted = true;
        }
    }
    if legacy.has("cloud_scroll_rate") {
        let r = &legacy["cloud_scroll_rate"];
        let mut scroll = Vec2::new(r.at(0).as_f32(), r.at(1).as_f32()) - Vec2::splat(10.0);
        if legacy.has("enable_cloud_scroll") {
            let enabled = &legacy["enable_cloud_scroll"];
            if !enabled.at(0).as_bool() {
                scroll.x = 0.0;
            }
            if !enabled.at(1).as_bool() {
                scroll.y = 0.0;
            }
        }
        out.insert("cloud_scroll_rate", eep::llsd_vec2(scroll));
        converted = true;
    }
    if legacy.has("star_brightness") {
        out.insert("star_brightness", Llsd::Real(legacy["star_brightness"].as_f64() * 250.0));
        converted = true;
    }
    if legacy.has("sunlight_color") {
        // LLColor4(...).getValue(): four components
        let c = &legacy["sunlight_color"];
        out.insert(
            "sunlight_color",
            Llsd::Array(
                (0..4)
                    .map(|i| Llsd::Real(if i < c.len() { c.at(i).as_f64() } else { 1.0 }))
                    .collect(),
            ),
        );
        converted = true;
    }
    for key in ["planet_radius", "sky_bottom_radius", "sky_top_radius", "sun_arc_radians"] {
        if legacy.has(key) {
            out.insert(key, Llsd::Real(legacy[key].as_f64()));
            converted = true;
        }
    }
    if legacy.has("east_angle") && legacy.has("sun_angle") {
        // counter-clockwise from Windlight's clockwise east angle; the moon
        // was diametrically opposed to the sun
        let azimuth = -legacy["east_angle"].as_f32();
        let altitude = legacy["sun_angle"].as_f32();
        out.insert("sun_rotation", eep::llsd_quat(azimuth_altitude_quat(azimuth, altitude)));
        out.insert(
            "moon_rotation",
            eep::llsd_quat(azimuth_altitude_quat(azimuth + std::f32::consts::PI, -altitude)),
        );
        converted = true;
    }
    converted.then_some(out)
}

/// LLSettingsWater::translateLegacySettings on LLSettingsWater::defaults().
fn translate_legacy_water(legacy: &Llsd) -> Option<Llsd> {
    let mut out = WaterFrame::default().settings_llsd();
    out.insert(SCALE_ABOVE.0, SCALE_ABOVE.1);
    out.insert(SCALE_BELOW.0, SCALE_BELOW.1);
    let mut converted = false;
    for (old, new) in [
        ("blurMultiplier", "blur_multiplier"),
        ("waterFogDensity", "water_fog_density"),
        ("underWaterFogMod", "underwater_fog_mod"),
        ("fresnelOffset", "fresnel_offset"),
        ("fresnelScale", "fresnel_scale"),
        ("scaleAbove", "scale_above"),
        ("scaleBelow", "scale_below"),
    ] {
        if legacy.has(old) {
            out.insert(new, Llsd::Real(legacy[old].as_f64()));
            converted = true;
        }
    }
    if legacy.has("waterFogColor") {
        out.insert("water_fog_color", color3(&legacy["waterFogColor"]));
        converted = true;
    }
    if legacy.has("normalMap") {
        out.insert("normal_map", Llsd::Uuid(legacy["normalMap"].as_uuid()));
        converted = true;
    }
    if legacy.has("normScale") {
        out.insert("normal_scale", color3(&legacy["normScale"]));
        converted = true;
    }
    for (old, new) in [("wave1Dir", "wave1_direction"), ("wave2Dir", "wave2_direction")] {
        if legacy.has(old) {
            let w = &legacy[old];
            out.insert(new, eep::llsd_vec2(Vec2::new(w.at(0).as_f32(), w.at(1).as_f32())));
            converted = true;
        }
    }
    converted.then_some(out)
}

/// Item permissions as settings flags (LLFloaterEditEnvironmentBase::
/// onAssetLoaded): what the editor may do with the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rights {
    pub save: bool,
    pub copy: bool,
    pub modify: bool,
    pub transfer: bool,
}

impl Default for Rights {
    fn default() -> Self {
        Rights::FREE
    }
}

impl Rights {
    /// Settings without an item (a snapshot, an import).
    pub const FREE: Rights = Rights {
        save: true,
        copy: true,
        modify: true,
        transfer: true,
    };

    /// FLAG_NOSAVE wins over the item's permissions.
    pub fn from_item(owner_mask: u32, flags: u32) -> Rights {
        if flags & FLAG_NOSAVE != 0 {
            return Rights {
                save: false,
                copy: false,
                modify: false,
                transfer: false,
            };
        }
        Rights {
            save: true,
            copy: owner_mask & 0x8000 != 0,
            modify: owner_mask & 0x4000 != 0,
            transfer: owner_mask & 0x2000 != 0,
        }
    }

    /// LLFloaterEditExtDayCycle::setEditDayCycle: what a day without an
    /// item allows (the parcel's or region's day, an import, the default
    /// day), from its own flags.
    pub fn from_flags(flags: u32) -> Rights {
        let save = flags & FLAG_NOSAVE == 0;
        Rights {
            save,
            copy: save && flags & FLAG_NOCOPY == 0,
            modify: save && flags & FLAG_NOMOD == 0,
            transfer: save && flags & FLAG_NOTRANS == 0,
        }
    }

    /// The flags written into the settings (setFlag / clearFlag).
    pub fn apply(self, doc: &mut SettingsDoc) {
        let f = self.flags(doc.flags());
        doc.set_flags(f);
    }

    /// `flags` with the permission flags of these rights.
    pub fn flags(self, flags: u32) -> u32 {
        let mut f = flags & !(FLAG_NOCOPY | FLAG_NOMOD | FLAG_NOTRANS);
        if !self.copy {
            f |= FLAG_NOCOPY;
        }
        if !self.modify {
            f |= FLAG_NOMOD;
        }
        if !self.transfer {
            f |= FLAG_NOTRANS;
        }
        f
    }
}

/// An inventory item's name, corrected (LLInventoryObject::correctInventoryName:
/// no `|`, at most DB_INV_ITEM_NAME_STR_LEN bytes), "Unnamed" when empty.
pub fn correct_item_name(name: &str) -> String {
    let mut out: String = name.chars().filter(|c| *c != '|' && !c.is_control()).collect();
    out = out.trim().to_owned();
    while out.len() > 63 {
        out.pop();
    }
    if out.is_empty() { "Unnamed".to_owned() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn sky_doc() -> SettingsDoc {
        SettingsDoc::from_llsd(llsd_map! {
            "type" => "sky",
            "name" => "Test",
            "gamma" => 1.0,
            "rayleigh_config" => Llsd::Array(vec![llsd_map! {"exp_term" => 1.0}]),
            "haze_density" => 0.5,
            "legacy_haze" => llsd_map! {"blue_density" => Llsd::Array(vec![0.1.into(), 0.2.into(), 0.3.into()])},
        })
        .expect("a sky")
    }

    #[test]
    fn unknown_keys_survive_an_edit() {
        let mut doc = sky_doc();
        let before = doc.sky();
        let mut after = before;
        after.gamma = 1.8;
        after.haze_horizon = 0.7;
        assert!(doc.edit_sky(&before, &after));
        let m = doc.llsd_ref();
        assert!(m.has("rayleigh_config"), "untouched keys stay");
        assert!((m["gamma"].as_f32() - 1.8).abs() < 1e-6);
        // an edited haze value lives in legacy_haze
        assert!((m["legacy_haze"]["haze_horizon"].as_f32() - 0.7).abs() < 1e-6);
        assert!(!m.has("haze_horizon"));
        // an untouched outer haze value is still read (get_float)
        assert!((doc.sky().haze_density - 0.5).abs() < 1e-6);
        assert!((doc.sky().blue_density.y - 0.2).abs() < 1e-6);
        // nothing changed: nothing written
        let snapshot = doc.clone();
        let s = doc.sky();
        assert!(!doc.edit_sky(&s, &s));
        assert_eq!(doc, snapshot);
    }

    #[test]
    fn probe_ambiance_makes_the_sky_non_classic() {
        let mut doc = sky_doc();
        let before = doc.sky();
        assert!(before.can_auto_adjust);
        let mut after = before;
        after.probe_ambiance = 0.4;
        after.can_auto_adjust = false;
        doc.edit_sky(&before, &after);
        let back = doc.sky();
        assert!(!back.can_auto_adjust && (back.probe_ambiance - 0.4).abs() < 1e-6);
    }

    #[test]
    fn the_asset_is_notation_with_its_header_and_reads_back() {
        let mut doc = sky_doc();
        doc.set_extra(MOISTURE_LEVEL, 0.6);
        let data = doc.to_asset();
        assert!(data.starts_with(b"<? llsd/notation ?>\n"));
        let back = SettingsDoc::from_asset(&data).expect("readable");
        assert_eq!(back.name(), "Test");
        assert!((back.extra(MOISTURE_LEVEL) - 0.6).abs() < 1e-6);
        assert!(back.llsd_ref().has("rayleigh_config"));
    }

    #[test]
    fn water_edits_write_their_keys() {
        let mut doc = SettingsDoc::snapshot_water(&WaterFrame::default(), "Snapshot water (new)");
        assert_eq!(doc.kind(), SettingsKind::Water);
        let before = doc.water();
        let mut after = before;
        after.fog_density = 9.0;
        after.wave1 = Vec2::new(-1.0, 0.5);
        assert!(doc.edit_water(&before, &after));
        let w = doc.water();
        assert_eq!(w.fog_density, 9.0);
        assert_eq!(w.wave1, Vec2::new(-1.0, 0.5));
        assert!((doc.extra(SCALE_BELOW) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn windlight_presets_are_translated() {
        let sky = br#"<?xml version="1.0" ?><llsd><map>
            <key>ambient</key><array><real>0.3</real><real>0.6</real><real>0.9</real><real>0.3</real></array>
            <key>haze_density</key><array><real>1.5</real><real>0</real><real>0</real><real>1</real></array>
            <key>gamma</key><array><real>1.2</real><real>0</real><real>0</real><real>1</real></array>
            <key>star_brightness</key><real>0.5</real>
            <key>cloud_scroll_rate</key><array><real>10.5</real><real>11</real></array>
            <key>enable_cloud_scroll</key><array><boolean>1</boolean><boolean>0</boolean></array>
            <key>east_angle</key><real>0</real>
            <key>sun_angle</key><real>0.5</real>
        </map></llsd>"#;
        let doc = SettingsDoc::import_legacy_sky(sky, "Nacon's Sunset").expect("converted");
        let s = doc.sky();
        assert_eq!(doc.name(), "Nacon's Sunset");
        assert!((s.ambient.y - 0.6).abs() < 1e-6);
        assert!((s.haze_density - 1.5).abs() < 1e-6);
        assert!((s.gamma - 1.2).abs() < 1e-6);
        assert!((s.star_brightness - 125.0).abs() < 1e-4);
        assert!((s.cloud_scroll_rate.x - 0.5).abs() < 1e-6 && s.cloud_scroll_rate.y == 0.0);
        assert!((s.sun_direction().z - 0.5f32.sin()).abs() < 1e-5);
        assert!(s.moon_direction().z < 0.0);
        let water = br#"<llsd><map><key>waterFogDensity</key><real>16</real>
            <key>wave1Dir</key><array><real>0.5</real><real>-0.5</real></array></map></llsd>"#;
        let w = SettingsDoc::import_legacy_water(water, "Murky").expect("converted").water();
        assert_eq!(w.fog_density, 16.0);
        assert_eq!(w.wave1, Vec2::new(0.5, -0.5));
        assert!(SettingsDoc::import_legacy_sky(b"<llsd><map></map></llsd>", "x").is_none());
        assert_eq!(legacy_preset_name("Nacon%27s%20Sunset"), "Nacon's Sunset");
    }

    #[test]
    fn rights_follow_the_item_and_nosave() {
        let r = Rights::from_item(0x8000 | 0x2000, 0);
        assert!(r.save && r.copy && !r.modify && r.transfer);
        let mut doc = sky_doc();
        r.apply(&mut doc);
        assert_eq!(doc.flags(), FLAG_NOMOD);
        assert!(!Rights::from_item(0x7fffffff, FLAG_NOSAVE).save);
        let day = Rights::from_flags(FLAG_NOMOD);
        assert!(day.save && day.copy && !day.modify && day.transfer);
        assert_eq!(Rights::from_flags(FLAG_NOSAVE), Rights::from_item(0, FLAG_NOSAVE));
        assert_eq!(correct_item_name(" a|b "), "ab");
        assert_eq!(correct_item_name(""), "Unnamed");
    }
}
