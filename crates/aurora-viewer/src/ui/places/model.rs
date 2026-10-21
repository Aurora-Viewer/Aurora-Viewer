//! What Aurora's "Places" lists show (`aurora.rs`): the landmarks of the
//! Landmarks and Favorites folders as places, one per landmark asset (a
//! favourite is a copy of a landmark in the Favorites folder, as the
//! "add_to_favbar" entry of LLPanelPlaces::onOverflowMenuItemClicked makes
//! it, indra/newview/llpanelplaces.cpp, originally LGPL 2.1), their last
//! visit read from the saved teleport history, and the places of the history
//! that no landmark keeps. Pure: the window draws what these functions give.

use crate::world::inventory::Inventory;
use crate::world::landmarks::{Landmarks, Resolved};
use crate::world::tphistory::HistoryItem;
use crate::world::worldmap::WorldMap;
use glam::{DVec3, Vec3};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// FT_LANDMARK and FT_FAVORITE.
pub const FOLDER_LANDMARKS: i32 = 3;
pub const FOLDER_FAVORITES: i32 = 23;
/// AT_LANDMARK.
const ASSET_LANDMARK: i32 = 3;
/// A teleport that arrived this close to a landmark (horizontally) was a
/// visit of it: a parcel's landing point puts the arrival away from the
/// landmark's own position.
pub const VISIT_RADIUS: f64 = 64.0;
/// It also has to be this close in height: a floor above or a skybox over
/// the landmark is another spot of the parcel, which the history keeps as
/// its own place (#759).
pub const VISIT_HEIGHT: f64 = 5.0;

/// How the landmarks are ordered (Preferences › Aurora keeps the choice).
pub use crate::settings::PlacesSort;

/// One line of the lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A landmark, by its asset: its copies are one place.
    Landmark(Uuid),
    /// An entry of the saved teleport history, by its index.
    Visit(usize),
}

/// A place as the lists and the preview show it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    pub name: String,
    /// The landmark item shown: the one outside Favorites, else the
    /// favourite.
    pub item: Option<Uuid>,
    pub asset: Option<Uuid>,
    /// Its copy in the Favorites folder.
    pub favorite: Option<Uuid>,
    /// The folder of `item` when it is not in Favorites.
    pub folder: Option<Uuid>,
    /// The landmark's notes (its item's description).
    pub note: String,
    /// The item's thumbnail (nil without one).
    pub thumbnail: Uuid,
    /// When the landmark was made (seconds since the epoch, 0 unknown).
    pub created: i64,
    pub global: Option<DVec3>,
    /// The region, once the map knows it ("" until then).
    pub region: String,
    /// SimAccess of the region, once known.
    pub rating: Option<u8>,
    /// Date of the last visit (seconds since the epoch).
    pub last_visit: Option<f64>,
    /// The history entry of that visit.
    pub history: Option<usize>,
}

impl Place {
    pub fn is_landmark(&self) -> bool {
        self.item.is_some()
    }

    /// The position in the region, for "Region (x, y, z)".
    pub fn region_pos(&self) -> Option<Vec3> {
        self.global
            .map(|g| Vec3::new(g.x.rem_euclid(256.0) as f32, g.y.rem_euclid(256.0) as f32, g.z as f32))
    }

    /// The text the filter looks in: name, region and notes.
    pub fn matches(&self, filter: &str) -> bool {
        let f = filter.trim().to_lowercase();
        f.is_empty()
            || self.name.to_lowercase().contains(&f)
            || self.region.to_lowercase().contains(&f)
            || self.note.to_lowercase().contains(&f)
    }
}

/// Every place, built once per frame from the inventory, the landmark
/// assets, the map and the history.
#[derive(Debug, Default)]
pub struct Catalog {
    /// The landmarks, one per asset, in the inventory's order.
    pub landmarks: Vec<Place>,
    /// The history entries no landmark keeps, newest first, one per title.
    pub visits: Vec<(usize, Place)>,
    /// History entry → the landmark it was a visit of (its asset).
    pub owner: HashMap<usize, Uuid>,
    /// Landmark assets to read.
    pub wanted_assets: Vec<Uuid>,
    /// Map cells (region grid x, y) whose region is not known yet.
    pub wanted_cells: Vec<(i64, i64)>,
}

impl Catalog {
    pub fn landmark(&self, asset: Uuid) -> Option<&Place> {
        self.landmarks.iter().find(|p| p.asset == Some(asset))
    }

    /// The place of a line.
    pub fn place(&self, key: Key, history: &[HistoryItem]) -> Option<Place> {
        match key {
            Key::Landmark(asset) => self.landmark(asset).cloned(),
            Key::Visit(i) => {
                let entry = history.get(i)?;
                match self.owner.get(&i).and_then(|a| self.landmark(*a)) {
                    // a visit of a landmark: the landmark, at that date and
                    // at the spot of the arrival
                    Some(lm) => Some(Place {
                        global: Some(entry.global),
                        last_visit: Some(entry.date),
                        history: Some(i),
                        ..lm.clone()
                    }),
                    None => Some(visit_place(i, entry)),
                }
            }
        }
    }
}

/// "Parcel, Region" (LLAgentUI::LOCATION_FORMAT_NORMAL) as name and region;
/// a title without a comma is the region alone.
pub fn split_title(title: &str) -> (String, String) {
    match title.rsplit_once(", ") {
        Some((name, region)) => (name.to_owned(), region.to_owned()),
        None => (title.to_owned(), title.to_owned()),
    }
}

fn visit_place(i: usize, entry: &HistoryItem) -> Place {
    let (name, region) = split_title(&entry.title);
    Place {
        name,
        region,
        global: Some(entry.global),
        last_visit: Some(entry.date),
        history: Some(i),
        ..Default::default()
    }
}

/// The landmark items under a system folder, its sub-folders too.
fn items_under(inv: &Inventory, kind: i32) -> Vec<Uuid> {
    let Some(root) = crate::world::appearance::system_folder(inv, kind) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(folder) = stack.pop() {
        let Some(f) = inv.folders.get(&folder).filter(|_| seen.insert(folder)) else {
            continue;
        };
        out.extend(
            f.items
                .iter()
                .filter(|id| inv.items.get(*id).is_some_and(|it| it.asset_type == ASSET_LANDMARK)),
        );
        // children in reverse so the first one is read first
        stack.extend(f.children.iter().rev().copied());
    }
    out
}

/// Build the catalog. `want_assets`: ask for the landmark assets not read
/// yet (the window is on screen).
pub fn catalog(inv: &Inventory, landmarks: &Landmarks, map: &WorldMap, history: &[HistoryItem], want_assets: bool) -> Catalog {
    let mut cat = Catalog::default();
    let mut by_asset: HashMap<Uuid, usize> = HashMap::new();
    let mut cells = HashSet::new();
    // the Landmarks folder first: its item is the one a place shows
    let favorite_items = items_under(inv, FOLDER_FAVORITES);
    let favorites: HashSet<Uuid> = favorite_items.iter().copied().collect();
    for id in items_under(inv, FOLDER_LANDMARKS).into_iter().chain(favorite_items) {
        let Some(it) = inv.items.get(&id) else { continue };
        let in_favorites = favorites.contains(&id);
        // a landmark with a nil asset is its own place
        let key = if it.asset_id.is_nil() { id } else { it.asset_id };
        if let Some(&n) = by_asset.get(&key) {
            let place = &mut cat.landmarks[n];
            if in_favorites && place.favorite.is_none() {
                place.favorite = Some(id);
            }
            continue;
        }
        let global = match landmarks.peek(it.asset_id) {
            Resolved::Ready { global, .. } => Some(global),
            Resolved::Pending => {
                if want_assets && !it.asset_id.is_nil() {
                    cat.wanted_assets.push(it.asset_id);
                }
                None
            }
            Resolved::Failed => None,
        };
        let sim = global.and_then(|g| map.sim_at_global(g.x, g.y).map(|(_, s)| (s.name.clone(), s.access)));
        if sim.is_none()
            && let Some(g) = global
        {
            cells.insert(((g.x / 256.0).floor() as i64, (g.y / 256.0).floor() as i64));
        }
        by_asset.insert(key, cat.landmarks.len());
        cat.landmarks.push(Place {
            name: it.name.clone(),
            item: Some(id),
            asset: Some(key),
            favorite: in_favorites.then_some(id),
            folder: (!in_favorites).then_some(it.parent),
            note: it.desc.clone(),
            thumbnail: it.thumbnail,
            created: it.created_at,
            global,
            region: sim.as_ref().map(|s| s.0.clone()).unwrap_or_default(),
            rating: sim.map(|s| s.1),
            ..Default::default()
        });
    }
    // the visits: each history entry goes to the nearest landmark of its
    // region within VISIT_RADIUS and VISIT_HEIGHT
    for (i, entry) in history.iter().enumerate() {
        let nearest = cat
            .landmarks
            .iter()
            .enumerate()
            .filter_map(|(n, p)| {
                let g = p.global?;
                let d = (g.truncate() - entry.global.truncate()).length();
                (d <= VISIT_RADIUS && (g.z - entry.global.z).abs() <= VISIT_HEIGHT).then_some((n, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match nearest {
            Some((n, _)) => {
                let place = &mut cat.landmarks[n];
                if place.last_visit.is_none_or(|d| entry.date > d) {
                    place.last_visit = Some(entry.date);
                    place.history = Some(i);
                }
                if let Some(asset) = place.asset {
                    cat.owner.insert(i, asset);
                }
            }
            None => cat.visits.push((i, visit_place(i, entry))),
        }
    }
    // newest first, one per title (the history keeps one entry per title
    // within 5 m; farther ones are other spots of a big parcel)
    cat.visits
        .sort_by(|a, b| b.1.last_visit.unwrap_or(0.0).total_cmp(&a.1.last_visit.unwrap_or(0.0)));
    let mut titles = HashSet::new();
    cat.visits.retain(|(i, _)| titles.insert(history[*i].title.clone()));
    for (_, v) in &mut cat.visits {
        if let Some(g) = v.global {
            match map.sim_at_global(g.x, g.y) {
                Some((_, s)) => {
                    v.region.clone_from(&s.name);
                    v.rating = Some(s.access);
                }
                None => {
                    cells.insert(((g.x / 256.0).floor() as i64, (g.y / 256.0).floor() as i64));
                }
            }
        }
    }
    cat.wanted_cells = cells.into_iter().collect();
    cat.wanted_cells.sort_unstable();
    cat
}

/// Order the landmarks: last visit first (never visited ones after, newest
/// landmark first), by name, or newest landmark first.
pub fn sort(places: &mut [Place], by: PlacesSort) {
    let name = |p: &Place| p.name.to_lowercase();
    match by {
        PlacesSort::Visit => places.sort_by(|a, b| {
            b.last_visit
                .unwrap_or(f64::MIN)
                .total_cmp(&a.last_visit.unwrap_or(f64::MIN))
                .then_with(|| b.created.cmp(&a.created))
                .then_with(|| name(a).cmp(&name(b)))
        }),
        PlacesSort::Name => places.sort_by_key(name),
        PlacesSort::Created => places.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| name(a).cmp(&name(b)))),
    }
}

/// The favourites, in the order of the favourites bar (`world/favorites.rs`):
/// `order` holds the items of the Favorites folder.
pub fn favorites(cat: &Catalog, order: &[Uuid]) -> Vec<Place> {
    let mut out: Vec<Place> = order
        .iter()
        .filter_map(|item| cat.landmarks.iter().find(|p| p.favorite == Some(*item)))
        .cloned()
        .collect();
    // a favourite the order does not know yet (just arrived) comes last
    for p in &cat.landmarks {
        if p.favorite.is_some() && !out.iter().any(|o| o.asset == p.asset) {
            out.push(p.clone());
        }
    }
    out
}

/// A folder of the Landmarks tree, as the rail and the chips list it.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderRow {
    pub id: Uuid,
    pub name: String,
    pub depth: usize,
}

/// The folders under the Landmarks folder, each level by name.
pub fn folders(inv: &Inventory) -> Vec<FolderRow> {
    let mut out = Vec::new();
    if let Some(root) = crate::world::appearance::system_folder(inv, FOLDER_LANDMARKS) {
        add_folders(inv, root, 0, &mut out, &mut HashSet::new());
    }
    out
}

fn add_folders(inv: &Inventory, parent: Uuid, depth: usize, out: &mut Vec<FolderRow>, seen: &mut HashSet<Uuid>) {
    let Some(f) = inv.folders.get(&parent).filter(|_| seen.insert(parent)) else {
        return;
    };
    let mut kids: Vec<_> = f.children.iter().filter_map(|c| inv.folders.get(c)).collect();
    kids.sort_by_key(|k| k.info.name.to_lowercase());
    for k in kids {
        out.push(FolderRow {
            id: k.info.id,
            name: k.info.name.clone(),
            depth,
        });
        add_folders(inv, k.info.id, depth + 1, out, seen);
    }
}

/// "Today, 21:40", "Yesterday, 23:05", else "16 Oct." (Second Life time,
/// like the days of the conversations).
pub fn visit_label(date: f64, now: f64) -> String {
    use crate::world::chat_log::format::Slt;
    let at = |t: f64| std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(t.max(0.0));
    let (d, today, yesterday) = (Slt::of(at(date)), Slt::of(at(now)), Slt::of(at(now - 86_400.0)));
    let same = |a: &Slt, b: &Slt| (a.year, a.month, a.day) == (b.year, b.month, b.day);
    let time = format!("{:02}:{:02}", d.hour, d.minute);
    if same(&d, &today) {
        aurora_i18n::tr!("ui-places-visit-today", time = time).to_owned()
    } else if same(&d, &yesterday) {
        aurora_i18n::tr!("ui-places-visit-yesterday", time = time).to_owned()
    } else {
        let month = crate::ui::land::months()[(d.month.clamp(1, 12) - 1) as usize];
        aurora_i18n::tr!("ui-places-visit-day", day = d.day.to_string(), month = month.to_string()).to_owned()
    }
}

/// "Created on 3 Oct. 2026" (Second Life time).
pub fn created_label(secs: i64) -> String {
    use crate::world::chat_log::format::Slt;
    if secs <= 0 {
        return aurora_i18n::tr!("status-unknown").to_owned();
    }
    let d = Slt::of(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64));
    let month = crate::ui::land::months()[(d.month.clamp(1, 12) - 1) as usize];
    aurora_i18n::tr!(
        "ui-places-created-on",
        day = d.day.to_string(),
        month = month.to_string(),
        year = d.year.to_string()
    )
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::MapBlock;
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(n: u128, parent: u128, name: &str, t: i32) -> InvFolder {
        InvFolder {
            id: id(n),
            parent: id(parent),
            name: name.into(),
            type_default: t,
            version: 1,
            ..Default::default()
        }
    }

    fn landmark(n: u128, parent: u128, name: &str, asset: u128, created: i64) -> InvItem {
        InvItem {
            id: id(n),
            parent: id(parent),
            name: name.into(),
            asset_type: 3,
            inv_type: 3,
            asset_id: id(asset),
            created_at: created,
            ..Default::default()
        }
    }

    /// Landmarks (10) with Amis (11) and Art (12) under it, Favorites (20).
    fn inventory() -> Inventory {
        let mut inv = Inventory::default();
        let contents = |f: InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>| FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        };
        let root = folder(1, 0, "My Inventory", 8);
        let lms = folder(10, 1, "Landmarks", FOLDER_LANDMARKS);
        let amis = folder(11, 10, "Amis", -1);
        let art = folder(12, 10, "art", -1);
        let fav = folder(20, 1, "Favorites", FOLDER_FAVORITES);
        inv.root = root.id;
        inv.apply(vec![contents(root, vec![lms.clone(), fav.clone()], Vec::new())]);
        inv.apply(vec![contents(
            lms,
            vec![amis.clone(), art.clone()],
            vec![landmark(100, 10, "Jardin", 1000, 300)],
        )]);
        inv.apply(vec![contents(amis, Vec::new(), vec![landmark(101, 11, "Café", 1001, 200)])]);
        inv.apply(vec![contents(art, Vec::new(), vec![landmark(102, 12, "Galerie", 1002, 100)])]);
        // the café is also a favourite (a copy, same asset); the plage is a
        // favourite only
        inv.apply(vec![contents(
            fav,
            Vec::new(),
            vec![landmark(200, 20, "Café", 1001, 250), landmark(201, 20, "Plage", 1003, 50)],
        )]);
        inv
    }

    /// The landmark assets point at (x, y) of the region at grid (1000, 1000).
    fn resolved() -> Landmarks {
        let region = id(0x5E61);
        let mut lm = Landmarks::default();
        lm.set_local_region(region, aurora_net::origin_to_handle(256_000, 256_000));
        for (asset, x, y) in [(1000, 10.0, 10.0), (1001, 100.0, 100.0), (1002, 200.0, 200.0), (1003, 240.0, 20.0)] {
            let text = format!("Landmark version 2\nregion_id {region}\nlocal_pos {x} {y} 22\n");
            lm.on_data(id(asset), Some(text.as_bytes()));
        }
        lm
    }

    fn map() -> WorldMap {
        let mut m = WorldMap::default();
        m.apply_blocks(vec![MapBlock {
            x: 1000,
            y: 1000,
            name: "Brumevale".into(),
            access: 13,
            region_flags: 0,
            water_height: 20,
            agents: 0,
            map_image_id: Uuid::nil(),
            size_x: 256,
            size_y: 256,
        }]);
        m
    }

    fn visit(title: &str, x: f64, y: f64, date: f64) -> HistoryItem {
        HistoryItem {
            title: title.into(),
            global: DVec3::new(256_000.0 + x, 256_000.0 + y, 22.0),
            date,
            slurl: String::new(),
        }
    }

    #[test]
    fn a_landmark_and_its_favourite_copy_are_one_place() {
        let cat = catalog(&inventory(), &resolved(), &map(), &[], false);
        let names: Vec<&str> = cat.landmarks.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Jardin", "Café", "Galerie", "Plage"]);
        let cafe = cat.landmark(id(1001)).expect("café");
        assert_eq!(cafe.item, Some(id(101)), "the copy outside Favorites is shown");
        assert_eq!(cafe.favorite, Some(id(200)));
        assert_eq!(cafe.folder, Some(id(11)));
        let plage = cat.landmark(id(1003)).expect("plage");
        assert_eq!((plage.item, plage.favorite, plage.folder), (Some(id(201)), Some(id(201)), None));
        assert_eq!(plage.region, "Brumevale");
        assert_eq!(plage.region_pos(), Some(Vec3::new(240.0, 20.0, 22.0)));
    }

    #[test]
    fn unread_landmarks_are_asked_for_once_shown() {
        let cat = catalog(&inventory(), &Landmarks::default(), &map(), &[], true);
        assert_eq!(cat.wanted_assets.len(), 4);
        assert!(cat.landmarks.iter().all(|p| p.global.is_none() && p.region.is_empty()));
        let cat = catalog(&inventory(), &Landmarks::default(), &map(), &[], false);
        assert!(cat.wanted_assets.is_empty());
        // read, but the region unknown: its cell is asked for
        let cat = catalog(&inventory(), &resolved(), &WorldMap::default(), &[], false);
        assert_eq!(cat.wanted_cells, vec![(1000, 1000)]);
    }

    #[test]
    fn visits_go_to_the_nearest_landmark_the_rest_stay_visits() {
        let history = vec![
            visit("Jardin, Brumevale", 30.0, 12.0, 10.0),
            visit("Café, Brumevale", 110.0, 95.0, 20.0),
            visit("Jardin, Brumevale", 12.0, 10.0, 30.0),
            visit("Place de l'Horloge, Brumevale", 160.0, 30.0, 40.0),
            visit("Place de l'Horloge, Brumevale", 165.0, 32.0, 50.0),
            visit("Clairvent", 60.0, 230.0, 5.0),
        ];
        let cat = catalog(&inventory(), &resolved(), &map(), &history, false);
        let jardin = cat.landmark(id(1000)).expect("jardin");
        assert_eq!((jardin.last_visit, jardin.history), (Some(30.0), Some(2)));
        assert_eq!(cat.landmark(id(1001)).and_then(|p| p.last_visit), Some(20.0));
        assert_eq!(cat.landmark(id(1002)).and_then(|p| p.last_visit), None);
        assert_eq!(cat.owner.get(&0), Some(&id(1000)));
        // the clock twice is one visit, the newest; the region alone keeps
        // its name as both
        let visits: Vec<(usize, &str, &str)> = cat.visits.iter().map(|(i, p)| (*i, p.name.as_str(), p.region.as_str())).collect();
        assert_eq!(
            visits,
            [(4, "Place de l'Horloge", "Brumevale"), (5, "Clairvent", "Brumevale")],
            "the region of a known cell comes from the map"
        );
        // a history line of a landmark shows the landmark at that date, at
        // the spot of the arrival
        let line = cat.place(Key::Visit(0), &history).expect("line");
        assert_eq!((line.item, line.last_visit), (Some(id(100)), Some(10.0)));
        assert_eq!(line.global, Some(history[0].global));
        let line = cat.place(Key::Visit(3), &history).expect("line");
        assert!(!line.is_landmark());
    }

    #[test]
    fn a_visit_at_another_height_is_its_own_place() {
        // the landmark is at 22 m; the other spot of the parcel is a meter
        // beside it, 30 m higher (#759)
        let mut deck = visit("Plage, Brumevale", 241.0, 20.0, 20.0);
        deck.global.z = 52.0;
        let history = vec![visit("Plage, Brumevale", 240.0, 21.0, 10.0), deck];
        let cat = catalog(&inventory(), &resolved(), &map(), &history, false);
        assert_eq!(cat.owner.get(&0), Some(&id(1003)));
        assert_eq!(cat.owner.get(&1), None);
        assert_eq!(cat.landmark(id(1003)).and_then(|p| p.last_visit), Some(10.0));
        let line = cat.place(Key::Visit(1), &history).expect("line");
        assert!(!line.is_landmark());
        assert_eq!((line.name.as_str(), line.global), ("Plage", Some(history[1].global)));
        assert_eq!(cat.visits.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn sorting_and_filtering() {
        let history = vec![visit("Galerie, Brumevale", 200.0, 200.0, 99.0), visit("Café", 100.0, 100.0, 50.0)];
        let mut places = catalog(&inventory(), &resolved(), &map(), &history, false).landmarks;
        let names = |p: &[Place]| p.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        sort(&mut places, PlacesSort::Visit);
        assert_eq!(names(&places), ["Galerie", "Café", "Jardin", "Plage"]);
        sort(&mut places, PlacesSort::Name);
        assert_eq!(names(&places), ["Café", "Galerie", "Jardin", "Plage"]);
        sort(&mut places, PlacesSort::Created);
        assert_eq!(names(&places), ["Jardin", "Café", "Galerie", "Plage"]);
        let mut cafe = places[1].clone();
        cafe.note = "Soirées jazz".into();
        assert!(cafe.matches("JAZZ") && cafe.matches("brume") && cafe.matches(" ") && !cafe.matches("plage"));
    }

    #[test]
    fn favourites_follow_the_bar_order() {
        let cat = catalog(&inventory(), &resolved(), &map(), &[], false);
        let names = |p: Vec<Place>| p.into_iter().map(|p| p.name).collect::<Vec<_>>();
        assert_eq!(names(favorites(&cat, &[id(201), id(200)])), ["Plage", "Café"]);
        assert_eq!(names(favorites(&cat, &[id(200)])), ["Café", "Plage"], "an unknown one goes last");
    }

    #[test]
    fn folder_tree_by_name() {
        let rows = folders(&inventory());
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("Amis", 0), ("art", 0)]);
    }

    #[test]
    fn titles_split_into_name_and_region() {
        assert_eq!(split_title("Café, Lumen Bay"), ("Café".into(), "Lumen Bay".into()));
        assert_eq!(split_title("Un, deux, Trois"), ("Un, deux".into(), "Trois".into()));
        assert_eq!(split_title("Clairvent"), ("Clairvent".into(), "Clairvent".into()));
    }

    #[test]
    fn visit_labels() {
        // 2026-10-19 21:40 SLT (PDT, UTC-7)
        let now = (aurora_net::profile::days_from_civil(2026, 10, 20) * 86_400 + 4 * 3600 + 40 * 60) as f64;
        assert_eq!(
            visit_label(now, now),
            aurora_i18n::tr!("ui-places-visit-today", time = "21:40").as_str()
        );
        assert_eq!(
            visit_label(now - 86_400.0 - 3600.0, now),
            aurora_i18n::tr!("ui-places-visit-yesterday", time = "20:40").as_str()
        );
        let month = crate::ui::land::months()[9];
        assert_eq!(
            visit_label(now - 3.0 * 86_400.0, now),
            aurora_i18n::tr!("ui-places-visit-day", day = "16", month = month).as_str()
        );
        assert_eq!(created_label(0), aurora_i18n::tr!("status-unknown"));
    }
}
