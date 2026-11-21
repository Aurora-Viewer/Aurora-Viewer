//! The Director's saved tracks and montages as files: what a montage file
//! holds, how an imported file is checked, what an export carries, and the
//! names they are saved under. Aurora's own: the Legacy Viewer has no
//! camera path tool. The files are written and read by threads of
//! `app/director_library.rs`; everything here is pure.
//!
//! A montage is a list of shots, each a saved track named by its name, played
//! one after the other with a cut between them (`director_montage.rs`). Its
//! file sits next to the tracks, in the `montages` folder of the account's
//! `director` folder: JSON, `MONTAGE_VERSION`. The tracks' own files are not
//! changed (`camera::track::TrackFile`, version 2).
//!
//! What a shared file may hold: a track keeps where its camera stood and,
//! for a keyframe that looks at a target, the id of that avatar (an agent
//! id) or of that object (an object id of the world). None of them is an
//! asset id: they name no texture, mesh, animation nor any creator's work,
//! so the creators' permission rule (AGENTS.md) has nothing to hide in them
//! and they are kept, the export of a track then plays as it was laid. An
//! inventory item id (an animation a keyframe plays) is no asset id either.

use crate::camera::track::{self, TrackFile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The folder of the montages, inside the account's `director` folder.
pub const MONTAGES: &str = "montages";
/// The version of the montages' files.
pub const MONTAGE_VERSION: u32 = 1;
/// What a montage exported with its tracks says it is.
pub const BUNDLE_KIND: &str = "aurora-director-montage";
pub const BUNDLE_VERSION: u32 = 1;
/// A montage holds this many shots at most.
pub const MAX_SHOTS: usize = 100;
/// A track's file is never larger than this (200 keyframes take ~80 kB).
pub const MAX_TRACK_BYTES: u64 = 1024 * 1024;
/// A montage exported with its tracks: at most `MAX_SHOTS` tracks.
pub const MAX_BUNDLE_BYTES: u64 = 24 * 1024 * 1024;
/// Global metres: no grid is wider than this.
const MAX_COORDINATE: f64 = 1.0e10;

/// A shot of a montage: the saved track it plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShotFile {
    pub track: String,
}

/// A montage as its file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MontageFile {
    pub version: u32,
    pub shots: Vec<ShotFile>,
}

impl MontageFile {
    pub fn of(shots: &[String]) -> MontageFile {
        MontageFile {
            version: MONTAGE_VERSION,
            shots: shots.iter().map(|track| ShotFile { track: track.clone() }).collect(),
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.shots.iter().map(|shot| shot.track.clone()).collect()
    }
}

/// A montage exported with its tracks, to be imported elsewhere whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleFile {
    pub kind: String,
    pub version: u32,
    pub name: String,
    pub montage: MontageFile,
    pub tracks: BTreeMap<String, TrackFile>,
}

/// Why a file was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Larger than such a file can be.
    TooLarge,
    /// Not the JSON of a track nor of a montage.
    Unreadable,
    /// A version this viewer does not know.
    Version(u32),
    /// A value out of bounds: what it is.
    Invalid(&'static str),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::TooLarge => write!(f, "file too large"),
            Refused::Unreadable => write!(f, "not a track nor a montage"),
            Refused::Version(v) => write!(f, "version {v} unknown"),
            Refused::Invalid(what) => write!(f, "invalid {what}"),
        }
    }
}

/// A track's file read as data from outside: a version this viewer reads,
/// a keyframe count it can hold, and every number finite and in bounds.
/// (`Track::from_file` still puts the times in order when it is loaded.)
pub fn check_track(file: &TrackFile) -> Result<(), Refused> {
    if file.version == 0 || file.version > track::FILE_VERSION {
        return Err(Refused::Version(file.version));
    }
    if file.keys.is_empty() || file.keys.len() > track::MAX_KEYS {
        return Err(Refused::Invalid("keyframe count"));
    }
    for key in &file.keys {
        if !key.time.is_finite() || !(0.0..=track::MAX_TIME * 2.0).contains(&key.time) {
            return Err(Refused::Invalid("keyframe time"));
        }
        if !key.position.iter().all(|v| v.is_finite() && v.abs() < MAX_COORDINATE) {
            return Err(Refused::Invalid("keyframe position"));
        }
        let length: f32 = key.rotation.iter().map(|v| v * v).sum();
        if !key.rotation.iter().all(|v| v.is_finite()) || !(1e-6..=4.0).contains(&length) {
            return Err(Refused::Invalid("keyframe rotation"));
        }
        if !key.fov.is_finite() || key.fov <= 0.0 {
            return Err(Refused::Invalid("keyframe view angle"));
        }
    }
    Ok(())
}

/// A montage's file read as data from outside.
pub fn check_montage(file: &MontageFile) -> Result<(), Refused> {
    if file.version != MONTAGE_VERSION {
        return Err(Refused::Version(file.version));
    }
    if file.shots.len() > MAX_SHOTS {
        return Err(Refused::Invalid("shot count"));
    }
    if !file
        .shots
        .iter()
        .all(|shot| track::file_stem(&shot.track).as_deref() == Some(shot.track.as_str()))
    {
        return Err(Refused::Invalid("shot name"));
    }
    Ok(())
}

/// A track's file: the bytes of a saved or imported one.
pub fn read_track(bytes: &[u8]) -> Result<TrackFile, Refused> {
    if bytes.len() as u64 > MAX_TRACK_BYTES {
        return Err(Refused::TooLarge);
    }
    let file: TrackFile = serde_json::from_slice(bytes).map_err(|_| Refused::Unreadable)?;
    check_track(&file)?;
    Ok(file)
}

pub fn read_montage(bytes: &[u8]) -> Result<MontageFile, Refused> {
    if bytes.len() as u64 > MAX_TRACK_BYTES {
        return Err(Refused::TooLarge);
    }
    let file: MontageFile = serde_json::from_slice(bytes).map_err(|_| Refused::Unreadable)?;
    check_montage(&file)?;
    Ok(file)
}

/// What an imported file holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Imported {
    /// A track, under the name its file had (`fallback` when none).
    Track { name: String, file: TrackFile },
    /// A montage with the tracks it plays.
    Montage {
        name: String,
        montage: MontageFile,
        tracks: Vec<(String, TrackFile)>,
    },
}

/// Read an imported file (`file_name`: the name it had on the disk): a
/// montage exported with its tracks, else a track. Every part is checked;
/// one track that is not sound refuses the whole file.
pub fn read_import(bytes: &[u8], file_name: &str, fallback: &str) -> Result<Imported, Refused> {
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(Refused::TooLarge);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Refused::Unreadable)?;
    if value.get("kind").and_then(|k| k.as_str()) == Some(BUNDLE_KIND) {
        let bundle: BundleFile = serde_json::from_value(value).map_err(|_| Refused::Unreadable)?;
        if bundle.version != BUNDLE_VERSION {
            return Err(Refused::Version(bundle.version));
        }
        check_montage(&bundle.montage)?;
        if bundle.tracks.len() > MAX_SHOTS {
            return Err(Refused::Invalid("track count"));
        }
        let mut tracks = Vec::with_capacity(bundle.tracks.len());
        for (name, file) in bundle.tracks {
            if track::file_stem(&name).as_deref() != Some(name.as_str()) {
                return Err(Refused::Invalid("track name"));
            }
            check_track(&file)?;
            tracks.push((name, file));
        }
        let name = track::file_stem(&bundle.name).unwrap_or_else(|| fallback.to_owned());
        return Ok(Imported::Montage {
            name,
            montage: bundle.montage,
            tracks,
        });
    }
    if bytes.len() as u64 > MAX_TRACK_BYTES {
        return Err(Refused::TooLarge);
    }
    let file: TrackFile = serde_json::from_value(value).map_err(|_| Refused::Unreadable)?;
    check_track(&file)?;
    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // "Dance.aurora-montage" and the like keep their first part
    let stem = stem.split('.').next().unwrap_or_default().to_owned();
    let name = track::file_stem(&stem).unwrap_or_else(|| fallback.to_owned());
    Ok(Imported::Track { name, file })
}

/// A montage and the tracks it plays, as one file to share. A shot whose
/// track is missing stays in the montage (it is shown missing where it is
/// imported) and brings no track.
pub fn bundle(name: &str, montage: &MontageFile, tracks: Vec<(String, TrackFile)>) -> BundleFile {
    BundleFile {
        kind: BUNDLE_KIND.to_owned(),
        version: BUNDLE_VERSION,
        name: name.to_owned(),
        montage: montage.clone(),
        tracks: tracks.into_iter().collect(),
    }
}

/// `wanted`, or "wanted 2", "wanted 3"… when another file of the folder
/// (`taken`, compared without case, as Windows does) has that name.
pub fn unique_name(wanted: &str, taken: &[String]) -> String {
    let free = |name: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(name));
    if free(wanted) {
        return wanted.to_owned();
    }
    for n in 2..10_000 {
        let suffix = format!(" {n}");
        let base: String = wanted.chars().take(60 - suffix.chars().count()).collect();
        let name = format!("{}{suffix}", base.trim_end());
        if free(&name) {
            return name;
        }
    }
    let base: String = wanted.chars().take(27).collect();
    format!("{} {}", base.trim_end(), uuid::Uuid::new_v4().simple())
}

/// A track renamed: the shots that played it play it under its new name.
/// True when the montage changed.
pub fn rename_shots(montage: &mut MontageFile, from: &str, to: &str) -> bool {
    let mut changed = false;
    for shot in &mut montage.shots {
        if shot.track == from {
            shot.track = to.to_owned();
            changed = true;
        }
    }
    changed
}

/// The names of the JSON files of `folder`, sorted.
pub fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
        .filter_map(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// What a montage's shot is, as the window shows it (no info: no saved
/// track has its name).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotInfo {
    /// Its track: keyframes, and seconds it plays.
    Ready { keys: usize, seconds: f32 },
    /// Its file is there but cannot be used.
    Unusable,
}

impl ShotInfo {
    pub fn of(file: &TrackFile) -> ShotInfo {
        let seconds = file.keys.iter().map(|k| k.time).fold(0.0_f32, f32::max).clamp(0.0, track::MAX_TIME);
        ShotInfo::Ready {
            keys: file.keys.len(),
            seconds,
        }
    }

    /// The shot plays (a track of two keyframes at least): its seconds.
    pub fn plays(self) -> Option<f32> {
        match self {
            ShotInfo::Ready { keys, seconds } if keys >= 2 => Some(seconds),
            _ => None,
        }
    }
}

/// Seconds the whole montage lasts: the shots that play, end to end.
pub fn total_seconds(infos: impl IntoIterator<Item = Option<ShotInfo>>) -> f32 {
    infos.into_iter().flatten().filter_map(ShotInfo::plays).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::{KeyFile, Track};
    use crate::director::{Aim, Joint, Target};
    use glam::Vec3;

    fn track_file(keys: usize) -> TrackFile {
        let mut track = Track::default();
        for i in 0..keys {
            track.append(track::Pose::from_view(Vec3::new(i as f32, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0));
        }
        track.to_file((256_000, 256_000))
    }

    fn neck() -> Aim {
        Aim {
            target: Target::Own,
            joint: Joint::Neck,
        }
    }

    #[test]
    fn a_montage_file_comes_back_as_it_was_saved() {
        let montage = MontageFile::of(&["Opening".to_owned(), "Dance 2".to_owned()]);
        let text = serde_json::to_vec_pretty(&montage).expect("json");
        assert_eq!(read_montage(&text), Ok(montage.clone()));
        assert_eq!(montage.names(), ["Opening", "Dance 2"]);
        // another version, too many shots, a name no file can have
        let mut newer = montage.clone();
        newer.version = 2;
        assert_eq!(read_montage(&serde_json::to_vec(&newer).expect("json")), Err(Refused::Version(2)));
        let many = MontageFile::of(&vec!["a".to_owned(); MAX_SHOTS + 1]);
        assert_eq!(check_montage(&many), Err(Refused::Invalid("shot count")));
        let bad = MontageFile::of(&["../secret".to_owned()]);
        assert_eq!(check_montage(&bad), Err(Refused::Invalid("shot name")));
        assert_eq!(read_montage(b"{"), Err(Refused::Unreadable));
        assert_eq!(read_montage(&vec![b' '; MAX_TRACK_BYTES as usize + 1]), Err(Refused::TooLarge));
    }

    #[test]
    fn an_imported_track_is_checked_before_it_is_taken() {
        let good = track_file(3);
        let bytes = serde_json::to_vec(&good).expect("json");
        match read_import(&bytes, "C:/films/Rooftop pan.json", "Imported") {
            Ok(Imported::Track { name, file }) => {
                assert_eq!(name, "Rooftop pan");
                assert_eq!(file, good);
            }
            other => panic!("{other:?}"),
        }
        // the name of a file that leaves none: the fallback
        assert!(matches!(read_import(&bytes, "....json", "Imported"), Ok(Imported::Track { name, .. }) if name == "Imported"));
        // not finite, out of bounds, too many, a version unknown
        let refused = |edit: &dyn Fn(&mut TrackFile)| {
            let mut file = good.clone();
            edit(&mut file);
            // (serde_json writes a NaN as null, which reads as no number)
            let bytes = serde_json::to_vec(&file).expect("json");
            read_import(&bytes, "x.json", "x").err()
        };
        assert_eq!(refused(&|f| f.keys[1].time = -1.0), Some(Refused::Invalid("keyframe time")));
        assert_eq!(
            refused(&|f| f.keys[1].position[0] = 1e300),
            Some(Refused::Invalid("keyframe position"))
        );
        assert_eq!(
            refused(&|f| f.keys[1].rotation = [0.0; 4]),
            Some(Refused::Invalid("keyframe rotation"))
        );
        assert_eq!(refused(&|f| f.keys[1].fov = 0.0), Some(Refused::Invalid("keyframe view angle")));
        assert_eq!(refused(&|f| f.keys[0].position[2] = f64::NAN), Some(Refused::Unreadable));
        assert_eq!(refused(&|f| f.keys.clear()), Some(Refused::Invalid("keyframe count")));
        assert_eq!(refused(&|f| f.version = 9), Some(Refused::Version(9)));
        let many = track_file(1);
        let mut many = many.clone();
        many.keys = vec![many.keys[0].clone(); track::MAX_KEYS + 1];
        assert_eq!(check_track(&many), Err(Refused::Invalid("keyframe count")));
        // a checked file is a file `Track::from_file` takes
        assert!(Track::from_file(&good, (256_000, 256_000), neck()).is_some());
    }

    #[test]
    fn bad_input_is_refused_never_a_panic() {
        for bytes in [
            &b""[..],
            b"null",
            b"[]",
            b"42",
            b"{\"version\":2}",
            b"{\"version\":2,\"keys\":\"x\"}",
            b"{\"kind\":\"aurora-director-montage\"}",
            b"{\"kind\":\"aurora-director-montage\",\"version\":1,\"name\":\"m\",\"montage\":{\"version\":1,\"shots\":[]},\"tracks\":{\"a\":{}}}",
            "\u{feff}{}".as_bytes(),
        ] {
            assert!(read_import(bytes, "f.json", "x").is_err(), "{:?}", String::from_utf8_lossy(bytes));
        }
        // too large for a track, before it is even parsed
        let mut huge = b"{\"keys\":[".to_vec();
        huge.resize(MAX_TRACK_BYTES as usize + 10, b' ');
        assert_eq!(read_import(&huge, "f.json", "x"), Err(Refused::Unreadable));
        assert_eq!(read_track(&huge), Err(Refused::TooLarge));
        assert_eq!(
            read_import(&vec![b' '; MAX_BUNDLE_BYTES as usize + 1], "f.json", "x"),
            Err(Refused::TooLarge)
        );
        // numbers that JSON cannot hold finite
        let mut text = serde_json::to_string(&track_file(2)).expect("json");
        text = text.replacen("\"fov\":1.0", "\"fov\":1e999", 1);
        assert!(read_import(text.as_bytes(), "f.json", "x").is_err());
    }

    #[test]
    fn a_montage_travels_with_its_tracks() {
        let montage = MontageFile::of(&["A".to_owned(), "Gone".to_owned(), "A".to_owned()]);
        let exported = bundle("Party", &montage, vec![("A".to_owned(), track_file(2))]);
        let bytes = serde_json::to_vec(&exported).expect("json");
        match read_import(&bytes, "Party.json", "x") {
            Ok(Imported::Montage {
                name,
                montage: back,
                tracks,
            }) => {
                assert_eq!(name, "Party");
                assert_eq!(back, montage, "the missing shot is kept: shown missing");
                assert_eq!(tracks.len(), 1);
                assert_eq!(tracks[0].0, "A");
            }
            other => panic!("{other:?}"),
        }
        // the export holds the targets' ids (agent and object ids, never an
        // asset id) and nothing else of the account
        let mut file = track_file(2);
        file.keys[1].aim = Some(track::AimFile::Object {
            id: uuid::Uuid::from_u128(5).to_string(),
        });
        let text = serde_json::to_string(&bundle("P", &MontageFile::of(&["A".to_owned()]), vec![("A".to_owned(), file)])).expect("json");
        assert!(text.contains(&uuid::Uuid::from_u128(5).to_string()));
        assert!(!text.contains("asset") && !text.contains("agent"));
        // one bad track refuses the whole file
        let mut bad = exported.clone();
        bad.tracks.insert(
            "B".to_owned(),
            TrackFile {
                keys: vec![KeyFile {
                    fov: -1.0,
                    ..track_file(1).keys[0].clone()
                }],
                ..track_file(1)
            },
        );
        let bytes = serde_json::to_vec(&bad).expect("json");
        assert_eq!(read_import(&bytes, "x.json", "x"), Err(Refused::Invalid("keyframe view angle")));
        let mut bad = exported.clone();
        bad.tracks.insert("a/../b".to_owned(), track_file(2));
        let bytes = serde_json::to_vec(&bad).expect("json");
        assert_eq!(read_import(&bytes, "x.json", "x"), Err(Refused::Invalid("track name")));
        let mut bad = exported;
        bad.version = 7;
        assert_eq!(
            read_import(&serde_json::to_vec(&bad).expect("json"), "x.json", "x"),
            Err(Refused::Version(7))
        );
    }

    #[test]
    fn a_name_taken_gets_a_number() {
        let taken = ["Pan".to_owned(), "pan 2".to_owned()];
        assert_eq!(unique_name("Orbit", &taken), "Orbit");
        assert_eq!(unique_name("PAN", &taken), "PAN 3");
        let long = "x".repeat(60);
        let named = unique_name(&long, std::slice::from_ref(&long));
        assert!(named.ends_with(" 2") && named.chars().count() == 60, "{named}");
        assert_eq!(track::file_stem(&named).as_deref(), Some(named.as_str()));
    }

    #[test]
    fn a_renamed_track_is_followed_by_its_shots() {
        let mut montage = MontageFile::of(&["A".to_owned(), "B".to_owned(), "A".to_owned()]);
        assert!(rename_shots(&mut montage, "A", "Opening"));
        assert_eq!(montage.names(), ["Opening", "B", "Opening"]);
        assert!(!rename_shots(&mut montage, "Z", "Y"));
    }

    #[test]
    fn the_files_of_a_folder_are_listed_without_its_subfolders() {
        let folder = std::env::temp_dir().join(format!("aurora-director-lib-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(folder.join(MONTAGES)).expect("fixture");
        for name in ["b.json", "A.JSON", "notes.txt"] {
            std::fs::write(folder.join(name), b"{}").expect("fixture");
        }
        assert_eq!(names_in(&folder), ["A", "b"]);
        assert!(names_in(&folder.join("none")).is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_montage_lasts_the_shots_that_play() {
        let infos = [
            Some(ShotInfo::of(&track_file(3))),
            None,
            Some(ShotInfo::of(&track_file(1))),
            None,
            Some(ShotInfo::Unusable),
            Some(ShotInfo::of(&track_file(2))),
        ];
        assert_eq!(infos[0].and_then(ShotInfo::plays), Some(6.0));
        assert_eq!(infos[2].and_then(ShotInfo::plays), None, "one keyframe goes nowhere");
        assert_eq!(total_seconds(infos), 9.0);
    }
}
