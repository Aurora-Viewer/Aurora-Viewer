//! The constellation data built into the viewer
//! (`crates/aurora-viewer/assets/constellations/`, its README.md) and the
//! user's own folder over it (`constellations/` in the configuration
//! folder, or `AURORA_CONSTELLATION_DIR` for a launch).
//!
//! The data is cut into units, each taken whole from one of the two:
//!
//! ```text
//! cultures/<id>/            a culture        the user's folder of that id replaces the built-in one
//! sets/<id>/                an art set       likewise (none is built in)
//! lines.json, names.json    the western culture without a folder: one unit, the user's as
//!                           soon as their folder has either file
//! planets.json              the planets' names
//! credits.json              the root's credits: never replaced, both are read
//! ```
//!
//! A unit of the user's replaces the built-in one whole, never figure by
//! figure: a culture is one source's work, and mixing two would credit
//! neither rightly. A user unit counts once its folder holds a file the scan
//! reads (`.json`, `.webp`); an empty folder replaces nothing. A user
//! `cultures/western/` is a culture folder like any other: the scan then
//! reads it instead of the root pair, whichever layer the pair comes from.
//! `AURORA_CONSTELLATION_BUILTIN=0` leaves the built-in layer out for a
//! launch (the user's folder alone).
//!
//! Only the files the scan reads are built in: the JSON of the lines, names,
//! cultures, planets and credits. The data's records for people
//! (`index.json`, `provenance.json`, `drafts.json`) stay in the repository,
//! checked by the tests below; no picture is built in. The built-in bytes
//! are static (nothing is read from disk to show them) and go through the
//! same parsing and limits as the user's files, on the scan's thread.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// `(path inside the data folder, bytes)` of each built-in file.
macro_rules! built_in {
    ($($rel:literal),* $(,)?) => {
        &[$(($rel, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/constellations/", $rel)))),*]
    };
}

/// The built-in files, sorted by path (a test checks it against the folder).
pub static TABLE: &[(&str, &[u8])] = built_in![
    "credits.json",
    "cultures/chinese/credits.json",
    "cultures/chinese/culture.json",
    "cultures/chinese/lines.json",
    "cultures/chinese/names.json",
    "cultures/egyptian/credits.json",
    "cultures/egyptian/culture.json",
    "cultures/egyptian/lines.json",
    "cultures/egyptian/names.json",
    "cultures/inuit/credits.json",
    "cultures/inuit/culture.json",
    "cultures/inuit/lines.json",
    "cultures/inuit/names.json",
    "cultures/japanese/credits.json",
    "cultures/japanese/culture.json",
    "cultures/japanese/lines.json",
    "cultures/japanese/names.json",
    "cultures/korean/credits.json",
    "cultures/korean/culture.json",
    "cultures/korean/lines.json",
    "cultures/korean/names.json",
    "cultures/norse-edda/credits.json",
    "cultures/norse-edda/culture.json",
    "cultures/norse-edda/lines.json",
    "cultures/norse-edda/names.json",
    "lines.json",
    "names.json",
    "planets.json",
];

/// `AURORA_CONSTELLATION_BUILTIN=0` (or `off`, `false`): the built-in layer
/// left out for this launch.
pub fn builtin_on() -> bool {
    !matches!(
        std::env::var("AURORA_CONSTELLATION_BUILTIN").ok().as_deref().map(str::trim),
        Some("0" | "off" | "false")
    )
}

/// Where one file's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    /// In the user's folder, read when asked.
    File(PathBuf),
    /// Built into the viewer.
    Builtin(&'static [u8]),
}

impl Source {
    /// Its bytes, at most `max` (more is an error: the built-in data passes
    /// the same limits as a file).
    fn read(&self, max: u64) -> Result<Vec<u8>, String> {
        match self {
            Source::File(path) => super::catalogue::read_capped(path, max),
            Source::Builtin(b) if b.len() as u64 > max => Err(format!("larger than {} KiB", max >> 10)),
            Source::Builtin(b) => Ok(b.to_vec()),
        }
    }
}

/// The unit of the root pair (the western culture without a folder) and of
/// the planets' names.
pub const WESTERN_ROOT: &str = "western";
pub const PLANETS: &str = "planets";
const ROOT_PAIR: [&str; 2] = ["lines.json", "names.json"];
const ROOT_CREDITS: &str = "credits.json";

/// The unit a data path belongs to (`None`: the root's credits, which both
/// layers give, or a root file the scan does not read).
fn unit_of(rel: &str) -> Option<String> {
    let mut parts = rel.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(top @ ("sets" | "cultures")), Some(id), Some(_)) => Some(format!("{top}/{id}")),
        (Some(super::planet_names::FILE), None, None) => Some(PLANETS.to_owned()),
        (Some(file), None, None) if ROOT_PAIR.contains(&file) => Some(WESTERN_ROOT.to_owned()),
        _ => None,
    }
}

/// The data as the scan sees it: every file by its path inside the data
/// folder, each unit from one layer.
#[derive(Clone, Debug, Default)]
pub struct Data {
    /// The user's folder (it may not exist).
    dir: PathBuf,
    /// The built-in layer is used.
    builtin: bool,
    files: BTreeMap<String, Source>,
    /// `sets/<id>` and `cultures/<id>` folders, with files or not (an empty
    /// user folder is still a culture or a set of its own name).
    dirs: BTreeSet<String>,
    /// The root's `credits.json` files: the built-in one first.
    root_credits: Vec<Source>,
    /// The units taken from the user's folder.
    user_units: BTreeSet<String>,
}

/// The user folder's files the scan may read: the root's, and those of each
/// `cultures/<id>/` and `sets/<id>/` (`.json`, `.webp`), bounded like the
/// folder's signature (constellations_catalogue.rs `signature`).
fn user_files(dir: &Path) -> (BTreeMap<String, PathBuf>, BTreeSet<String>) {
    use super::catalogue::{MAX_SET_FIGURES, MAX_SETS, entries};
    let mut files = BTreeMap::new();
    let mut dirs = BTreeSet::new();
    for name in [ROOT_PAIR[0], ROOT_PAIR[1], super::planet_names::FILE, ROOT_CREDITS] {
        if dir.join(name).is_file() {
            files.insert(name.to_owned(), dir.join(name));
        }
    }
    for top in ["cultures", "sets"] {
        for (sub, m) in entries(&dir.join(top)).into_iter().take(MAX_SETS * 4) {
            if !m.is_dir() {
                continue;
            }
            dirs.insert(format!("{top}/{sub}"));
            let folder = dir.join(top).join(&sub);
            for (file, fm) in entries(&folder).into_iter().take(MAX_SET_FIGURES * 4) {
                if fm.is_file() && (file.ends_with(".json") || file.ends_with(".webp")) {
                    files.insert(format!("{top}/{sub}/{file}"), folder.join(&file));
                }
            }
        }
    }
    (files, dirs)
}

impl Data {
    /// The data the viewer reads: the built-in layer (unless
    /// `AURORA_CONSTELLATION_BUILTIN=0`) under the user's folder `dir`.
    pub fn of(dir: &Path) -> Data {
        Data::layered(dir, builtin_on().then_some(TABLE))
    }

    /// The user's folder alone (the built-in layer off).
    #[cfg(test)]
    pub fn folder(dir: &Path) -> Data {
        Data::layered(dir, None)
    }

    /// `table` (the built-in layer; `None`: off) under the user's folder
    /// `dir`: each unit the user's folder has replaces the table's whole.
    fn layered(dir: &Path, table: Option<&'static [(&'static str, &'static [u8])]>) -> Data {
        let (user, user_dirs) = user_files(dir);
        let mut data = Data {
            dir: dir.to_path_buf(),
            builtin: table.is_some(),
            user_units: user.keys().filter_map(|rel| unit_of(rel)).collect(),
            ..Data::default()
        };
        for &(rel, bytes) in table.unwrap_or_default() {
            match unit_of(rel) {
                None if rel == ROOT_CREDITS => data.root_credits.push(Source::Builtin(bytes)),
                Some(u) if !data.user_units.contains(&u) => {
                    data.files.insert(rel.to_owned(), Source::Builtin(bytes));
                }
                _ => {}
            }
        }
        for (rel, path) in user {
            if rel == ROOT_CREDITS {
                data.root_credits.push(Source::File(path));
            } else {
                data.files.insert(rel, Source::File(path));
            }
        }
        let folders: Vec<String> = data
            .files
            .keys()
            .filter_map(|rel| unit_of(rel).filter(|u| u.contains('/')))
            .collect();
        data.dirs.extend(folders);
        data.dirs.extend(user_dirs);
        data
    }

    /// The bytes of the file at `rel` (a path inside the data folder):
    /// `None` when the data has no such file, else at most `max` of them
    /// (more is an error).
    pub fn read(&self, rel: &str, max: u64) -> Option<Result<Vec<u8>, String>> {
        self.files.get(rel).map(|s| s.read(max))
    }

    /// The file at `rel` when it is one of the user's folder (pictures are
    /// read from there only: none is built in).
    pub fn path(&self, rel: &str) -> Option<&Path> {
        match self.files.get(rel) {
            Some(Source::File(path)) => Some(path),
            _ => None,
        }
    }

    /// The folders under `top` (`sets`, `cultures`): their names, sorted.
    pub fn subdirs(&self, top: &str) -> Vec<String> {
        let prefix = format!("{top}/");
        self.dirs
            .iter()
            .filter_map(|d| d.strip_prefix(&prefix))
            .map(str::to_owned)
            .collect()
    }

    /// The files directly in the folder `dir` (`sets/<id>`): names, sorted.
    pub fn files_in(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{dir}/");
        self.files
            .range(prefix.clone()..)
            .take_while(|(rel, _)| rel.starts_with(&prefix))
            .filter_map(|(rel, _)| rel.strip_prefix(&prefix).filter(|n| !n.contains('/')).map(str::to_owned))
            .collect()
    }

    /// The bytes of each of the root's `credits.json`, built-in first (an
    /// error for one past `max`).
    pub fn root_credits(&self, max: u64) -> Vec<Result<Vec<u8>, String>> {
        self.root_credits.iter().map(|s| s.read(max)).collect()
    }

    /// For the log: where a problem's file is (a problem starts with its
    /// path inside the data, `cultures/<id>/…: …`), the user's folder or the
    /// built-in data.
    pub fn origin_of(&self, problem: &str) -> String {
        let rel = problem.split(':').next().unwrap_or_default();
        let builtin = self.builtin && unit_of(rel).is_some_and(|u| !self.user_units.contains(&u));
        if builtin {
            "built-in data".to_owned()
        } else {
            self.dir.display().to_string()
        }
    }

    /// For the log: the layers read, and the units the user's folder gives.
    pub fn describe(&self) -> String {
        let units = self.user_units.iter().map(String::as_str).collect::<Vec<_>>().join(", ");
        match (self.builtin, units.is_empty()) {
            (false, _) => self.dir.display().to_string(),
            (true, true) => "the built-in data".to_owned(),
            (true, false) => format!("the built-in data and {} ({units})", self.dir.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::constellations::catalogue::tests::{folder, write};
    use crate::world::constellations::catalogue::{self, MAX_LINES_FILE};
    use serde_json::Value;

    const LINES: &str = r#"{"Ori":{"name":"Orion","label":[88.0,22.8],"lines":[{"a":[78.6,-8.2],"b":[81.1,-2.4]}]},
        "Cyg":{"name":"Cygnus","label":[310.0,40.0],"lines":[{"a":[305.0,40.0],"b":[310.0,45.0]}]}}"#;
    const OTHER: &str = r#"{"Aa":{"name":"Aa","label":[10.0,10.0],"lines":[{"a":[10.0,10.0],"b":[11.0,11.0]}]}}"#;

    /// A small built-in table: the root pair, a culture, the planets and
    /// the root's credits.
    fn builtin() -> &'static [(&'static str, &'static [u8])] {
        static TABLE: &[(&str, &[u8])] = &[
            ("credits.json", br#"{"credits":[{"holder":"Built Library"}]}"#),
            ("cultures/other/culture.json", br#"{"name":"Other"}"#),
            ("cultures/other/lines.json", OTHER.as_bytes()),
            ("lines.json", LINES.as_bytes()),
            ("names.json", br#"{"names":{"fr":{"Ori":"Orion built"}}}"#),
            ("planets.json", br#"{"mars":{"name":"Built Mars"}}"#),
        ];
        TABLE
    }

    fn scan(data: &Data) -> (catalogue::Catalogue, Vec<String>) {
        let mut problems = Vec::new();
        let c = catalogue::scan_data(data, &mut problems);
        (c, problems)
    }

    fn mars(data: &Data) -> Option<String> {
        let planets = super::super::planet_names::read(data, &mut Vec::new());
        let k = crate::world::planets::ephemeris::Body::ALL
            .iter()
            .position(|b| b.id() == "mars")
            .expect("Mars");
        planets[k].as_ref().map(|n| n.name.clone())
    }

    #[test]
    fn units_of_paths() {
        assert_eq!(unit_of("sets/a/Ori.json").as_deref(), Some("sets/a"));
        assert_eq!(unit_of("cultures/b/lines.json").as_deref(), Some("cultures/b"));
        assert_eq!(unit_of("lines.json").as_deref(), Some(WESTERN_ROOT));
        assert_eq!(unit_of("names.json").as_deref(), Some(WESTERN_ROOT));
        assert_eq!(unit_of("planets.json").as_deref(), Some(PLANETS));
        assert_eq!(unit_of("credits.json"), None);
        assert_eq!(unit_of("provenance.json"), None);
    }

    #[test]
    fn the_built_in_layer_alone_and_an_empty_user_folder() {
        let root = folder("layers-builtin");
        // an empty culture folder of the user's replaces nothing
        std::fs::create_dir_all(root.join("cultures/other")).expect("folder");
        let data = Data::layered(&root, Some(builtin()));
        assert!(data.user_units.is_empty());
        assert_eq!(data.describe(), "the built-in data");
        let (c, problems) = scan(&data);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.cultures.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["western", "other"]);
        let ori = c.figures.iter().find(|f| f.id == "Ori").expect("Orion");
        assert_eq!(ori.names.get("fr").map(String::as_str), Some("Orion built"));
        assert!(c.figures.iter().any(|f| f.id == "Aa"));
        assert_eq!(mars(&data).as_deref(), Some("Built Mars"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_user_culture_replaces_the_built_in_one_whole() {
        let root = folder("layers-culture");
        // the user's culture of the same id has Bb only: Aa is not filled
        // in from the built-in one, nor its culture.json
        write(
            &root,
            "cultures/other/lines.json",
            br#"{"Bb":{"name":"Bb","label":[20.0,20.0],"lines":[{"a":[20.0,20.0],"b":[21.0,21.0]}]}}"#,
        );
        let data = Data::layered(&root, Some(builtin()));
        assert_eq!(data.user_units.iter().collect::<Vec<_>>(), ["cultures/other"]);
        let (c, problems) = scan(&data);
        assert!(problems.is_empty(), "{problems:?}");
        let other = c.cultures.iter().position(|k| k.id == "other").expect("the culture");
        let ids: Vec<&str> = c.figures.iter().filter(|f| f.culture == other).map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["Bb"]);
        assert!(c.cultures[other].names.is_none(), "no built-in culture.json under the user's");
        // the western root pair and the planets stay built in
        assert!(c.figures.iter().any(|f| f.id == "Ori" && !f.geom.lines.is_empty()));
        assert_eq!(mars(&data).as_deref(), Some("Built Mars"));
        assert!(data.origin_of("cultures/other/lines.json: bad").ends_with("layers-culture"));
        assert_eq!(data.origin_of("lines.json: bad"), "built-in data");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_user_root_pair_and_planets_replace_the_built_in_ones() {
        let root = folder("layers-root");
        // names.json alone takes the whole root pair: no built-in lines.json
        write(&root, "names.json", br#"{"names":{"fr":{"Ori":"Orion mine"}}}"#);
        write(&root, "planets.json", br#"{"mars":{"name":"My Mars"}}"#);
        let data = Data::layered(&root, Some(builtin()));
        assert_eq!(
            data.user_units.iter().map(String::as_str).collect::<Vec<_>>(),
            [PLANETS, WESTERN_ROOT]
        );
        assert!(
            data.read("lines.json", MAX_LINES_FILE).is_none(),
            "the root pair is the user's, whole"
        );
        let (c, _) = scan(&data);
        assert!(!c.figures.iter().any(|f| f.id == "Ori"), "no western lines left");
        assert!(c.figures.iter().any(|f| f.id == "Aa"), "the built-in culture stays");
        assert_eq!(mars(&data).as_deref(), Some("My Mars"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_user_western_folder_wins_over_the_built_in_root_pair() {
        let root = folder("layers-western");
        write(
            &root,
            "cultures/western/lines.json",
            br#"{"Lyr":{"name":"Lyra","label":[280.0,36.0],"lines":[{"a":[279.0,38.0],"b":[281.0,33.0]}]}}"#,
        );
        let (c, _) = scan(&Data::layered(&root, Some(builtin())));
        let western: Vec<&str> = c.figures.iter().filter(|f| f.culture == 0).map(|f| f.id.as_str()).collect();
        assert_eq!(western, ["Lyr"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_root_credits_add_up() {
        let root = folder("layers-credits");
        write(&root, "credits.json", br#"{"credits":[{"holder":"My Source"}]}"#);
        let data = Data::layered(&root, Some(builtin()));
        assert!(data.user_units.is_empty(), "the root's credits replace no unit");
        let groups = super::super::credits::read(&data, &[], &[], &mut Vec::new());
        let holders: Vec<&str> = groups.iter().flat_map(|g| g.credits.iter().map(|c| c.holder.as_str())).collect();
        assert_eq!(holders, ["Built Library", "My Source"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_the_built_in_layer_the_user_folder_is_read_alone() {
        let root = folder("layers-off");
        let (c, _) = scan(&Data::folder(&root));
        assert!(c.is_empty(), "no folder, no built-in layer: nothing");
        write(&root, "lines.json", LINES.as_bytes());
        let data = Data::folder(&root);
        assert!(!data.builtin);
        let (c, _) = scan(&data);
        assert_eq!(c.figures.len(), 2);
        assert_eq!(data.describe(), root.display().to_string());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn oversize_built_in_entries_are_refused() {
        let big: &'static [u8] = Box::leak(vec![b' '; MAX_LINES_FILE as usize + 1].into_boxed_slice());
        let table: &'static [(&'static str, &'static [u8])] = Box::leak(vec![("cultures/big/lines.json", big)].into_boxed_slice());
        let root = folder("layers-big");
        let (c, problems) = scan(&Data::layered(&root, Some(table)));
        assert!(
            problems.iter().any(|p| p.starts_with("cultures/big/lines.json: larger than")),
            "{problems:?}"
        );
        assert!(c.figures.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    // The real built-in data.

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/constellations")
    }

    /// Every file of the data folder, by its path inside it.
    fn tree(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("the data folder reads").flatten() {
            let name = entry.file_name().into_string().expect("UTF-8 names");
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if entry.path().is_dir() {
                tree(&entry.path(), &rel, out);
            } else {
                out.insert(rel);
            }
        }
    }

    fn json(rel: &str) -> Value {
        let bytes = std::fs::read(assets().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    #[test]
    fn the_built_in_data_reads_whole() {
        let root = folder("layers-real");
        let data = Data::layered(&root, Some(TABLE));
        let (c, problems) = scan(&data);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.cultures.len(), 7, "{}", catalogue::summary(&c));
        assert!(
            c.sets.is_empty() && c.figures.iter().all(|f| f.drawings.is_empty()),
            "no picture is built in"
        );
        assert_eq!(c.figures.iter().filter(|f| f.culture == 0).count(), 88);
        let mut problems = Vec::new();
        let planets = super::super::planet_names::read(&data, &mut problems);
        assert!(
            planets.iter().all(Option::is_none),
            "planets.json names none: the catalogues' names"
        );
        // no name and no stroke timing is built in: a culture's figures
        // have none, the western ones the catalogues' (`constellation-*`)
        for f in &c.figures {
            assert!(f.names.is_empty() && f.name.is_empty(), "{}: a built-in name", f.id);
            assert_eq!(f.name_key.is_some(), f.culture == 0, "{}", f.id);
            assert!(
                f.geom.lines.iter().all(|l| (l.t0, l.t1) == (0.0, 1.0)),
                "{}: a built-in stroke timing",
                f.id
            );
        }
        let cultures: Vec<(String, String)> = c.cultures.iter().map(|k| (k.id.clone(), k.id.clone())).collect();
        let groups = super::super::credits::read(&data, &cultures, &[], &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(groups.len(), 1 + 6, "the root's credits and each culture's");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The table builds in exactly the folder's files the scan reads;
    /// `index.json` lists its cultures (and no set); every unit has its
    /// credits and origin records; NOTICE.md has one row per unit; the
    /// folder holds nothing but JSON and its README (no picture).
    #[test]
    fn the_index_the_folders_and_the_notice_agree() {
        let mut files = BTreeSet::new();
        tree(&assets(), "", &mut files);
        let read: BTreeSet<String> = files
            .iter()
            .filter(|rel| {
                let name = rel.rsplit('/').next().unwrap_or_default();
                ["lines.json", "names.json", "culture.json", "credits.json", "planets.json"].contains(&name)
            })
            .cloned()
            .collect();
        let table: Vec<&str> = TABLE.iter().map(|(rel, _)| *rel).collect();
        assert!(table.windows(2).all(|w| w[0] < w[1]), "TABLE sorted, no duplicate");
        assert_eq!(
            table.iter().map(|r| (*r).to_owned()).collect::<BTreeSet<_>>(),
            read,
            "TABLE and the folder differ"
        );
        let odd: Vec<&String> = files.iter().filter(|r| !r.ends_with(".json") && *r != "README.md").collect();
        assert!(odd.is_empty(), "only JSON and the README: {odd:?}");

        let units: BTreeSet<String> = table.iter().filter_map(|rel| unit_of(rel)).collect();
        let index = json("index.json");
        let listed = |key: &str| -> BTreeSet<String> {
            index[key]
                .as_array()
                .unwrap_or_else(|| panic!("index.json lists its {key}"))
                .iter()
                .map(|id| format!("{key}/{}", id.as_str().expect("an id")))
                .collect()
        };
        let folders: BTreeSet<String> = units.iter().filter(|u| u.contains('/')).cloned().collect();
        assert_eq!(listed("cultures"), folders, "index.json and the culture folders differ");
        assert!(listed("sets").is_empty(), "no art set is built in");
        assert!(units.contains(WESTERN_ROOT) && units.contains(PLANETS));
        for unit in &folders {
            for file in ["credits.json", "provenance.json", "lines.json"] {
                assert!(files.contains(&format!("{unit}/{file}")), "{unit}: no {file}");
            }
        }
        for file in ["credits.json", "provenance.json", "drafts.json"] {
            assert!(files.contains(file), "no root {file}");
        }

        let notice = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../NOTICE.md")).expect("NOTICE.md");
        let rows: Vec<&str> = notice
            .split("## Bundled assets")
            .nth(1)
            .and_then(|s| s.split("\n## ").next())
            .expect("the Bundled assets table")
            .lines()
            .filter(|l| l.contains("assets/constellations/"))
            .collect();
        let named: BTreeSet<String> = rows
            .iter()
            .filter_map(|row| {
                let at = row.split("`crates/aurora-viewer/assets/constellations/").nth(1)?;
                Some(match at.split('`').next()?.trim_end_matches('/') {
                    "lines.json" => WESTERN_ROOT.to_owned(),
                    "planets.json" => PLANETS.to_owned(),
                    other => other.to_owned(),
                })
            })
            .collect();
        assert_eq!(rows.len(), named.len(), "one NOTICE row per unit");
        assert_eq!(named, units, "NOTICE.md's rows and the data's units differ");
    }

    /// Each unit's `provenance.json` marks every figure of its lines, with
    /// known fields and values, none of a figure's `unknown`. (Names and
    /// readings may be `unknown`: the record says which.)
    #[test]
    fn every_built_in_unit_records_where_its_figures_come_from() {
        const FIELDS: [&str; 8] = ["drawing", "reveal", "lines", "timing", "label", "name_box", "name", "reading"];
        const ORIGINS: [&str; 5] = ["sourced", "derived", "adapted", "made", "unknown"];
        let units = TABLE.iter().filter_map(|(rel, _)| unit_of(rel)).filter(|u| u != PLANETS);
        for unit in units.collect::<BTreeSet<_>>() {
            let dir = if unit == WESTERN_ROOT { String::new() } else { format!("{unit}/") };
            let lines = json(&format!("{dir}lines.json"));
            let record = json(&format!("{dir}provenance.json"));
            let figures = record["figures"].as_object().unwrap_or_else(|| panic!("{unit}: no figures"));
            for id in lines.as_object().expect("lines.json is an object").keys() {
                let marks = figures.get(id).and_then(Value::as_object);
                let marks = marks.unwrap_or_else(|| panic!("{unit}: no record for {id}"));
                for (field, origin) in marks {
                    assert!(FIELDS.contains(&field.as_str()), "{unit}/{id}: field {field}");
                    let origin = origin.as_str().unwrap_or_default();
                    assert!(ORIGINS.contains(&origin) && origin != "unknown", "{unit}/{id}.{field}: {origin}");
                }
                assert!(!marks.contains_key("drawing"), "{unit}/{id}: a drawing is marked, none is built in");
            }
        }
    }

    /// No built-in part is marked `made` (no source content): every
    /// origin mark of every `provenance.json`, the planets' included.
    #[test]
    fn no_built_in_part_is_made_without_a_source() {
        fn marks<'a>(v: &'a Value, at: String, out: &mut Vec<(String, &'a str)>) {
            match v {
                Value::Object(m) => {
                    // (the record's own words, not marks)
                    for (k, x) in m.iter().filter(|(k, _)| !["about", "notes", "unit"].contains(&k.as_str())) {
                        marks(x, format!("{at}/{k}"), out);
                    }
                }
                Value::String(s) => out.push((at, s.as_str())),
                _ => {}
            }
        }
        let units = TABLE.iter().filter_map(|(rel, _)| unit_of(rel)).filter(|u| u != PLANETS);
        for unit in units.collect::<BTreeSet<_>>() {
            let dir = if unit == WESTERN_ROOT { String::new() } else { format!("{unit}/") };
            let record = json(&format!("{dir}provenance.json"));
            let mut found = Vec::new();
            marks(&record, unit.clone(), &mut found);
            assert!(!found.is_empty(), "{unit}: no mark");
            let made: Vec<&String> = found.iter().filter(|(_, m)| *m == "made").map(|(at, _)| at).collect();
            assert!(made.is_empty(), "made without a source: {made:?}");
        }
    }
}
