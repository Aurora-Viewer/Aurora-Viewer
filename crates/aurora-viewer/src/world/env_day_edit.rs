//! A day cycle being edited (the `mEditDay` of LLFloaterEditExtDayCycle,
//! newview/llfloatereditextdaycycle.cpp, and LLSettingsDay,
//! llinventory/llsettingsdaycycle.cpp, originally LGPL 2.1).
//!
//! Five tracks (0: water, 1: ground level sky, 2..=4: higher skies) of key
//! frames at a position of the day (0..1). Each frame is kept as its
//! settings map, untouched unless edited, and the map is rebuilt on save
//! (`frames` named by a hash of their content, `tracks` of key_keyframe /
//! key_name), as LLSettingsDay::getSettings does. The editing rules are the
//! floater's and its multi slider's: 20 frames at most per track
//! (max_sliders), positions on 0.005 steps (increment), two frames never
//! within 0.026 of each other, around the loop (overlap_threshold,
//! loop_overlap), and DEFAULT_FRAME_SLOP_FACTOR to tell whether a frame sits
//! at the time cursor.

use super::eep::{self, DayCycle, Settings, SkyFrame, WaterFrame};
use super::env_edit::SettingsDoc;
use aurora_llsd::Llsd;
use std::hash::{Hash, Hasher};

/// LLSettingsDay::TRACK_MAX.
pub const TRACKS: usize = 5;
pub const TRACK_WATER: usize = 0;
pub const TRACK_GROUND: usize = 1;
/// The frames multi slider's max_sliders.
pub const MAX_FRAMES: usize = 20;
/// LLSettingsDay::DEFAULT_FRAME_SLOP_FACTOR.
pub const FRAME_SLOP: f32 = 0.02501;
/// The multi sliders' increment and overlap_threshold.
pub const INCREMENT: f32 = 0.005;
pub const OVERLAP: f32 = 0.026;
/// DAY_CYCLE_PLAY_TIME_SECONDS: "Play" runs a whole day in a minute.
pub const PLAY_SECONDS: f32 = 60.0;

/// Distance between two positions around the day's loop.
pub fn loop_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs();
    d.min(1.0 - d)
}

/// A position on the sliders' steps, within the day.
pub fn snap(pos: f32) -> f32 {
    ((pos.clamp(0.0, 1.0) / INCREMENT).round() * INCREMENT).clamp(0.0, 1.0)
}

/// Why a track could not be cloned or loaded (TrackLoadMismatch,
/// TrackLoadFailed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackError {
    Mismatch,
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DayDoc {
    /// The day's own keys (name, type, flags…), without frames and tracks.
    map: Llsd,
    /// Key frames by track, sorted by position.
    pub tracks: [Vec<(f32, Llsd)>; TRACKS],
}

fn frame_type(track: usize) -> &'static str {
    if track == TRACK_WATER { "water" } else { "sky" }
}

impl DayDoc {
    /// A day cycle settings map (LLSettingsDay::initialize): keys naming a
    /// missing or mistyped frame are dropped, positions clamped to the day.
    pub fn from_llsd(v: Llsd) -> Option<DayDoc> {
        if !v.is_map() || v["type"].as_str() != "daycycle" {
            return None;
        }
        let mut tracks: [Vec<(f32, Llsd)>; TRACKS] = Default::default();
        for (t, track) in tracks.iter_mut().enumerate() {
            for key in v["tracks"].at(t).as_array() {
                let frame = &v["frames"][key["key_name"].as_str()];
                if !frame.is_map() || (frame.has("type") && frame["type"].as_str() != frame_type(t)) {
                    continue;
                }
                let mut frame = frame.clone();
                frame.insert("type", frame_type(t));
                let pos = key["key_keyframe"].as_f32();
                let pos = if pos.is_finite() { pos.clamp(0.0, 1.0) } else { 0.0 };
                match track.iter_mut().find(|(p, _)| *p == pos) {
                    Some(k) => k.1 = frame,
                    None => track.push((pos, frame)),
                }
            }
            track.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        let mut map = v;
        if let Llsd::Map(m) = &mut map {
            m.remove("frames");
            m.remove("tracks");
        }
        Some(DayDoc { map, tracks })
    }

    pub fn from_asset(data: &[u8]) -> Option<DayDoc> {
        Settings::decode_asset(data).and_then(DayDoc::from_llsd)
    }

    /// setEditDayCycle: an empty water or ground track gets a default frame
    /// at the middle of the day.
    pub fn ensure_defaults(&mut self) {
        if self.tracks[TRACK_WATER].is_empty() {
            let w = SettingsDoc::snapshot_water(&WaterFrame::default(), "");
            self.tracks[TRACK_WATER].push((0.5, without_name(w.into_llsd())));
        }
        if self.tracks[TRACK_GROUND].is_empty() {
            self.tracks[TRACK_GROUND].push((0.5, SkyFrame::default().settings_llsd()));
        }
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

    /// The settings map (LLSettingsDay::getSettings).
    pub fn to_llsd(&self) -> Llsd {
        let mut frames = aurora_llsd::Map::new();
        let mut tracks = Vec::with_capacity(TRACKS);
        for track in &self.tracks {
            let mut keys = Vec::with_capacity(track.len());
            for (pos, frame) in track {
                let text = aurora_llsd::to_notation(frame);
                let mut h = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut h);
                let name = h.finish().to_string();
                frames.insert(name.clone(), frame.clone());
                keys.push(aurora_llsd::llsd_map! {"key_keyframe" => *pos as f64, "key_name" => name});
            }
            tracks.push(Llsd::Array(keys));
        }
        let mut out = self.map.clone();
        out.insert("type", "daycycle");
        out.insert("frames", Llsd::Map(frames));
        out.insert("tracks", Llsd::Array(tracks));
        out
    }

    /// The asset to upload, in LLSD notation with its header.
    pub fn to_asset(&self) -> Vec<u8> {
        format!("<? llsd/notation ?>\n{}", aurora_llsd::to_notation(&self.to_llsd())).into_bytes()
    }

    /// The day as rendered.
    pub fn day(&self) -> DayCycle {
        DayCycle::from_settings(&self.to_llsd(), eep::DEFAULT_DAY_LENGTH, eep::DEFAULT_DAY_OFFSET)
    }

    /// The sky and water shown at `pos` while `track` is selected: the
    /// selected sky track (the ground when it is empty), or the last sky
    /// track for the water (reblendSettings keeps the sky blender's track).
    pub fn frames_at(&self, day: &DayCycle, pos: f32, sky_track: usize) -> (SkyFrame, WaterFrame) {
        let t = sky_track.clamp(TRACK_GROUND, TRACKS - 1);
        let t = if self.tracks[t].is_empty() { TRACK_GROUND } else { t };
        (day.sky_at(pos, t - 1), day.water_at(pos))
    }

    /// The frame of `track` within `slop` of `pos` (getSettingsNearKeyframe).
    pub fn near(&self, track: usize, pos: f32, slop: f32) -> Option<usize> {
        self.tracks[track]
            .iter()
            .enumerate()
            .filter(|(_, (p, _))| loop_distance(*p, pos) <= slop)
            .min_by(|a, b| loop_distance(a.1.0, pos).total_cmp(&loop_distance(b.1.0, pos)))
            .map(|(i, _)| i)
    }

    /// isAddingFrameAllowed: no frame at the cursor and room on the slider.
    pub fn can_add(&self, track: usize, pos: f32) -> bool {
        self.near(track, pos, FRAME_SLOP).is_none() && self.tracks[track].len() < MAX_FRAMES
    }

    fn insert(&mut self, track: usize, pos: f32, frame: Llsd) -> usize {
        let t = &mut self.tracks[track];
        let i = t.iter().position(|(p, _)| *p > pos).unwrap_or(t.len());
        t.insert(i, (pos, frame));
        i
    }

    /// onAddFrame: the settings shown at the cursor become a frame there.
    pub fn add_frame(&mut self, track: usize, pos: f32, sky_track: usize) -> Option<usize> {
        let pos = snap(pos);
        if !self.can_add(track, pos) {
            return None;
        }
        let (sky, water) = self.frames_at(&self.day(), pos, if track == TRACK_WATER { sky_track } else { track });
        let frame = if track == TRACK_WATER {
            without_name(SettingsDoc::snapshot_water(&water, "").into_llsd())
        } else {
            sky.settings_llsd()
        };
        Some(self.insert(track, pos, frame))
    }

    /// A loaded sky or water at the cursor: replaces the frame there, or is
    /// added (onAssetLoadedForInsertion).
    pub fn put_frame(&mut self, track: usize, pos: f32, frame: &SettingsDoc) -> Option<usize> {
        let want = if track == TRACK_WATER { "water" } else { "sky" };
        if frame.llsd_ref()["type"].as_str() != want {
            return None;
        }
        let pos = snap(pos);
        if let Some(i) = self.near(track, pos, FRAME_SLOP) {
            self.tracks[track].remove(i);
        } else if self.tracks[track].len() >= MAX_FRAMES {
            return None;
        }
        Some(self.insert(track, pos, without_name(frame.llsd_ref().clone())))
    }

    /// isRemovingFrameAllowed: the water and ground tracks keep a frame.
    pub fn can_remove(&self, track: usize) -> bool {
        let n = self.tracks[track].len();
        if track <= TRACK_GROUND { n > 1 } else { n > 0 }
    }

    pub fn remove_frame(&mut self, track: usize, index: usize) -> bool {
        if !self.can_remove(track) || index >= self.tracks[track].len() {
            return false;
        }
        self.tracks[track].remove(index);
        true
    }

    /// Whether `pos` is free for frame `index` (the multi slider refuses to
    /// bring two sliders within overlap_threshold, around the loop).
    fn free_for(&self, track: usize, index: Option<usize>, pos: f32) -> bool {
        self.tracks[track]
            .iter()
            .enumerate()
            .all(|(i, (p, _))| Some(i) == index || loop_distance(*p, pos) >= OVERLAP)
    }

    /// Drag a frame (moveTrackKeyframe). Returns its new index.
    pub fn move_frame(&mut self, track: usize, index: usize, pos: f32) -> Option<usize> {
        let pos = snap(pos);
        let (old, _) = *self.tracks[track].get(index)?;
        if (old - pos).abs() < f32::EPSILON || !self.free_for(track, Some(index), pos) {
            return None;
        }
        let (_, frame) = self.tracks[track].remove(index);
        Some(self.insert(track, pos, frame))
    }

    /// Shift-drag: a copy of the frame at `pos` (onFrameSliderCallback with
    /// mShiftCopyEnabled). Returns the copy's index.
    pub fn copy_frame(&mut self, track: usize, index: usize, pos: f32) -> Option<usize> {
        let pos = snap(pos);
        let frame = self.tracks[track].get(index)?.1.clone();
        if self.tracks[track].len() >= MAX_FRAMES || !self.free_for(track, None, pos) {
            return None;
        }
        Some(self.insert(track, pos, frame))
    }

    /// cloneTrack: water only to water; an empty source leaves the track.
    pub fn clone_track(&mut self, source: &DayDoc, from: usize, to: usize) -> Result<(), TrackError> {
        if (from == TRACK_WATER || to == TRACK_WATER) && from != to {
            return Err(TrackError::Mismatch);
        }
        let frames = source.tracks[from].clone();
        if frames.is_empty() {
            return Err(TrackError::Empty);
        }
        self.tracks[to] = frames;
        Ok(())
    }

    /// updateButtons' can_clone: another sky track has frames.
    pub fn can_clone(&self, track: usize) -> bool {
        track != TRACK_WATER && (TRACK_GROUND..TRACKS).any(|t| t != track && !self.tracks[t].is_empty())
    }

    /// onClearTrack: a higher sky track empties, the water and ground
    /// tracks keep their first frame.
    pub fn can_clear(&self, track: usize) -> bool {
        let n = self.tracks[track].len();
        if track > TRACK_GROUND { n > 0 } else { n > 1 }
    }

    pub fn clear_track(&mut self, track: usize) {
        if track > TRACK_GROUND {
            self.tracks[track].clear();
        } else {
            self.tracks[track].truncate(1);
        }
    }

    /// Skip forward / back: the next or previous frame of the track, around
    /// the loop (getUpperBoundFrame / getLowerBoundFrame).
    pub fn step(&self, track: usize, pos: f32, forward: bool) -> Option<f32> {
        let t = &self.tracks[track];
        if t.is_empty() {
            return None;
        }
        let half = INCREMENT / 2.0;
        Some(if forward {
            t.iter().map(|(p, _)| *p).find(|p| *p > pos + half).unwrap_or(t[0].0)
        } else {
            t.iter()
                .rev()
                .map(|(p, _)| *p)
                .find(|p| *p < pos - half)
                .unwrap_or(t[t.len() - 1].0)
        })
    }

    /// A frame as a sky or water document for the panels.
    pub fn frame_doc(&self, track: usize, index: usize) -> Option<SettingsDoc> {
        let mut frame = self.tracks[track].get(index)?.1.clone();
        frame.insert("type", frame_type(track));
        SettingsDoc::from_llsd(frame)
    }

    /// The panels changed a frame.
    pub fn set_frame(&mut self, track: usize, index: usize, doc: &SettingsDoc) {
        if let Some(k) = self.tracks[track].get_mut(index) {
            k.1 = without_name(doc.llsd_ref().clone());
        }
    }
}

/// Why a Windlight day could not be imported (WLImportFail's REASONS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyError {
    /// SettingImportFileError: a file could not be opened (its path).
    Open(String),
    /// SettingParseFileError: a file is not LLSD XML (its path).
    Parse(String),
    /// SettingValidationError: nothing could be converted (the preset).
    Invalid(String),
}

/// The folders next to a Windlight day's `days` folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolder {
    Skies,
    Water,
}

/// The file names read_legacy_preset_data tries for a preset, in order:
/// the name, LLURI::escape of it, then with its dashes escaped too.
pub fn legacy_file_names(name: &str) -> [String; 3] {
    let escaped = aurora_net::land::uri_escape(name);
    [
        format!("{name}.xml"),
        format!("{escaped}.xml"),
        format!("{}.xml", escaped.replace('-', "%2D")),
    ]
}

impl DayDoc {
    /// LLSettingsVODay::buildFromLegacyPreset: a Windlight day file (an
    /// array of [position, sky name]) becomes a day whose water is the
    /// `Default` preset of the water folder and whose ground track holds the
    /// skies of the skies folder. `read` returns a preset's bytes and path,
    /// or the path it could not open.
    pub fn import_legacy(
        day_xml: &[u8],
        day_path: &str,
        name: &str,
        read: &mut dyn FnMut(LegacyFolder, &str) -> Result<(Vec<u8>, String), String>,
    ) -> Result<DayDoc, LegacyError> {
        let keys = aurora_llsd::from_xml(day_xml).map_err(|_| LegacyError::Parse(day_path.to_owned()))?;
        let mut frames = aurora_llsd::Map::new();
        let (data, path) = read(LegacyFolder::Water, "Default").map_err(LegacyError::Open)?;
        aurora_llsd::from_xml(&data).map_err(|_| LegacyError::Parse(path))?;
        let water = SettingsDoc::import_legacy_water(&data, "Default").ok_or_else(|| LegacyError::Invalid("Default".into()))?;
        frames.insert("water:Default".into(), water.into_llsd());
        let mut sky_track = Vec::new();
        for key in keys.as_array() {
            let sky = key.at(1).to_string_value();
            sky_track.push(aurora_llsd::llsd_map! {"key_keyframe" => key.at(0).as_f64(), "key_name" => format!("sky:{sky}")});
            if frames.contains_key(&format!("sky:{sky}")) {
                continue;
            }
            let (data, path) = read(LegacyFolder::Skies, &sky).map_err(LegacyError::Open)?;
            aurora_llsd::from_xml(&data).map_err(|_| LegacyError::Parse(path))?;
            let shown = super::env_edit::legacy_preset_name(&sky);
            let frame = SettingsDoc::import_legacy_sky(&data, &shown).ok_or(LegacyError::Invalid(shown))?;
            frames.insert(format!("sky:{sky}"), frame.into_llsd());
        }
        let water_track = Llsd::Array(vec![aurora_llsd::llsd_map! {"key_keyframe" => 0.0, "key_name" => "water:Default"}]);
        let day = aurora_llsd::llsd_map! {
            "type" => "daycycle",
            "name" => super::env_edit::legacy_preset_name(name),
            "frames" => Llsd::Map(frames),
            "tracks" => Llsd::Array(vec![water_track, Llsd::Array(sky_track)]),
        };
        DayDoc::from_llsd(day).ok_or_else(|| LegacyError::Invalid(name.to_owned()))
    }
}

/// Frames carry no name of their own in a day.
fn without_name(mut v: Llsd) -> Llsd {
    if let Llsd::Map(m) = &mut v {
        m.remove("name");
    }
    v
}

/// "[PRCNT]%" and, when the day length is known, the time of day
/// (onOpen's p0..p4 and current_time labels).
pub fn time_label(pos: f32, day_length: Option<f64>) -> String {
    let pct = (pos * 100.0).round() as i32;
    match day_length {
        Some(len) if len > 0.0 => {
            let secs = (pos as f64 * len).round() as i64;
            format!("{pct}% ({}:{:02})", secs / 3600, (secs / 60) % 60)
        }
        _ => format!("{pct}%"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn day() -> DayDoc {
        let sky = |g: f64| llsd_map! {"type" => "sky", "gamma" => g, "density_profile_kept" => 1};
        let v = llsd_map! {
            "type" => "daycycle",
            "name" => "Test day",
            "frames" => llsd_map! {
                "a" => sky(1.0),
                "b" => sky(2.0),
                "w" => llsd_map! {"type" => "water", "water_fog_density" => 4.0},
                "bad" => llsd_map! {"type" => "water"},
            },
            "tracks" => Llsd::Array(vec![
                Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "w"}]),
                Llsd::Array(vec![
                    llsd_map! {"key_keyframe" => 0.5, "key_name" => "b"},
                    llsd_map! {"key_keyframe" => 0.0, "key_name" => "a"},
                    llsd_map! {"key_keyframe" => 0.25, "key_name" => "bad"},
                ]),
            ]),
        };
        DayDoc::from_llsd(v).expect("a day")
    }

    #[test]
    fn frames_round_trip_and_mistyped_keys_are_dropped() {
        let d = day();
        assert_eq!(d.name(), "Test day");
        assert_eq!(d.tracks[TRACK_GROUND].len(), 2, "the water frame on a sky track is dropped");
        assert_eq!(d.tracks[TRACK_GROUND][0].0, 0.0);
        let back = DayDoc::from_asset(&d.to_asset()).expect("readable");
        assert_eq!(back.tracks, d.tracks);
        assert!(back.tracks[TRACK_GROUND][1].1.has("density_profile_kept"));
        let rendered = d.day();
        assert!((rendered.sky_at(0.5, 0).gamma - 2.0).abs() < 1e-6);
        assert!((rendered.water_at(0.3).fog_density - 4.0).abs() < 1e-6);
    }

    #[test]
    fn adding_respects_the_slop_the_steps_and_the_limit() {
        let mut d = day();
        assert!(!d.can_add(TRACK_GROUND, 0.51), "a frame sits at 0.5");
        assert!(!d.can_add(TRACK_GROUND, 0.99), "0.0 is within the slop around the loop");
        let i = d.add_frame(TRACK_GROUND, 0.2513, 1).expect("added");
        assert_eq!(d.tracks[TRACK_GROUND][i].0, 0.25, "snapped to 0.005");
        // the new frame is the blend at its position
        let g = SkyFrame::default();
        assert!(d.frame_doc(TRACK_GROUND, i).expect("doc").sky().gamma > g.gamma);
        let mut full = day();
        for k in 0..30 {
            full.add_frame(4, k as f32 / 30.0, 1);
        }
        assert_eq!(full.tracks[4].len(), MAX_FRAMES);
    }

    #[test]
    fn moving_copying_removing_and_clearing() {
        let mut d = day();
        assert_eq!(d.move_frame(TRACK_GROUND, 1, 0.02), None, "too close to 0.0");
        assert_eq!(d.move_frame(TRACK_GROUND, 1, 0.98), None, "too close to 0.0 around the loop");
        let i = d.move_frame(TRACK_GROUND, 1, 0.7).expect("moved");
        assert_eq!(d.tracks[TRACK_GROUND][i].0, 0.7);
        let c = d.copy_frame(TRACK_GROUND, i, 0.3).expect("copied");
        assert_eq!(d.tracks[TRACK_GROUND].len(), 3);
        assert_eq!(d.tracks[TRACK_GROUND][c].1, d.tracks[TRACK_GROUND][2].1);
        assert!(d.remove_frame(TRACK_GROUND, 0));
        assert!(d.can_clear(TRACK_GROUND));
        d.clear_track(TRACK_GROUND);
        assert_eq!(d.tracks[TRACK_GROUND].len(), 1, "the ground keeps a frame");
        assert!(!d.remove_frame(TRACK_GROUND, 0));
        assert!(!d.remove_frame(TRACK_WATER, 0), "the water keeps a frame");
        assert_eq!(d.step(TRACK_GROUND, 0.5, true), Some(d.tracks[TRACK_GROUND][0].0));
    }

    #[test]
    fn tracks_clone_between_skies_only() {
        let src = day();
        let mut d = day();
        assert_eq!(d.clone_track(&src, TRACK_GROUND, TRACK_WATER), Err(TrackError::Mismatch));
        assert_eq!(d.clone_track(&src, 3, 2), Err(TrackError::Empty));
        assert!(!d.can_clone(TRACK_GROUND), "no other sky track has frames");
        d.clone_track(&src, TRACK_GROUND, 3).expect("cloned");
        assert_eq!(d.tracks[3], src.tracks[TRACK_GROUND]);
        assert!(d.can_clone(TRACK_GROUND) && d.can_clear(3));
        // a higher track is shown once it has frames
        let (sky, _) = d.frames_at(&d.day(), 0.5, 3);
        assert!((sky.gamma - 2.0).abs() < 1e-6);
    }

    #[test]
    fn windlight_days_take_their_skies_and_the_default_water() {
        let day = br#"<llsd><array>
            <array><real>0</real><string>A-12AM</string></array>
            <array><real>0.5</real><string>Noon</string></array>
            <array><real>0.75</real><string>A-12AM</string></array>
        </array></llsd>"#;
        let sky = |gamma: f32| {
            format!("<llsd><map><key>gamma</key><array><real>{gamma}</real><real>0</real><real>0</real><real>1</real></array></map></llsd>")
                .into_bytes()
        };
        let mut asked = Vec::new();
        let mut read = |folder: LegacyFolder, name: &str| -> Result<(Vec<u8>, String), String> {
            asked.push((folder, name.to_owned()));
            match (folder, name) {
                (LegacyFolder::Water, "Default") => Ok((
                    b"<llsd><map><key>waterFogDensity</key><real>16</real></map></llsd>".to_vec(),
                    "water/Default.xml".into(),
                )),
                (LegacyFolder::Skies, "A-12AM") => Ok((sky(1.5), "skies/A%2D12AM.xml".into())),
                (LegacyFolder::Skies, "Noon") => Ok((sky(2.5), "skies/Noon.xml".into())),
                _ => Err(format!("{name}.xml")),
            }
        };
        let d = DayDoc::import_legacy(day, "days/Mine.xml", "My%20Day", &mut read).expect("imported");
        assert_eq!(d.name(), "My Day");
        assert_eq!(d.tracks[TRACK_GROUND].len(), 3);
        assert_eq!(
            asked.iter().filter(|(_, n)| n == "A-12AM").count(),
            1,
            "a sky used twice is read once"
        );
        let rendered = d.day();
        assert!((rendered.sky_at(0.5, 0).gamma - 2.5).abs() < 1e-6);
        assert!((rendered.water_at(0.1).fog_density - 16.0).abs() < 1e-6);
        // a missing sky names its file, a broken day file its path
        let mut missing = |folder: LegacyFolder, name: &str| -> Result<(Vec<u8>, String), String> {
            match folder {
                LegacyFolder::Water => Ok((
                    b"<llsd><map><key>waterFogDensity</key><real>2</real></map></llsd>".to_vec(),
                    "w".into(),
                )),
                LegacyFolder::Skies => Err(format!("skies/{name}.xml")),
            }
        };
        assert_eq!(
            DayDoc::import_legacy(day, "d", "x", &mut missing),
            Err(LegacyError::Open("skies/A-12AM.xml".into()))
        );
        assert_eq!(
            DayDoc::import_legacy(b"not xml", "days/x.xml", "x", &mut missing),
            Err(LegacyError::Parse("days/x.xml".into()))
        );
        assert_eq!(
            legacy_file_names("A-12AM"),
            ["A-12AM.xml".to_owned(), "A-12AM.xml".to_owned(), "A%2D12AM.xml".to_owned()]
        );
        assert_eq!(legacy_file_names("Nacon's")[1], "Nacon%27s.xml");
    }

    #[test]
    fn defaults_fill_empty_tracks_and_labels() {
        let mut d = DayDoc::from_llsd(llsd_map! {"type" => "daycycle"}).expect("empty day");
        d.ensure_defaults();
        assert_eq!(d.tracks[TRACK_WATER].len(), 1);
        assert_eq!(d.tracks[TRACK_GROUND][0].0, 0.5);
        assert_eq!(time_label(0.25, None), "25%");
        assert_eq!(time_label(0.5, Some(14400.0)), "50% (2:00)");
    }
}
