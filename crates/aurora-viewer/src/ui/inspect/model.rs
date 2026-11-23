//! The inspected prims gathered into objects, one per linkset, for the
//! Outfit and Creators lists: the body area of the attachment point, the
//! figures of the whole object, the heavy badge, the share of the outfit and
//! the sort orders. Pure logic, drawn by `list.rs`.

use super::Row;
use uuid::Uuid;

/// Where an object is worn, by attachment point (the AttachmentPoint ids of
/// avatar_lad.xml); objects in the world are `World`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    Head,
    Body,
    Arms,
    Legs,
    Hud,
    Other,
    World,
}

impl Area {
    /// The sections of the Outfit list, head to toe.
    pub const ORDER: [Area; 7] = [Area::Head, Area::Body, Area::Arms, Area::Legs, Area::Hud, Area::Other, Area::World];

    pub fn of_point(point: u8) -> Area {
        match point {
            0 => Area::World,
            // skull, mouth, chin, ears, eyes, nose; jaw, alt ears and eyes, tongue
            2 | 11..=17 | 47..=52 => Area::Head,
            // chest, spine, pelvis, stomach, pecs, neck, avatar center, tail, wings, groin
            1 | 9 | 10 | 28..=30 | 39 | 40 | 43..=46 | 53 => Area::Body,
            // shoulders, hands, arms, ring fingers
            3..=6 | 18..=21 | 41 | 42 => Area::Arms,
            // feet, hips, legs, hind feet
            7 | 8 | 22..=27 | 54 | 55 => Area::Legs,
            31..=38 => Area::Hud,
            _ => Area::Other,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Area::Head => aurora_i18n::tr!("inspect-area-head"),
            Area::Body => aurora_i18n::tr!("inspect-area-body"),
            Area::Arms => aurora_i18n::tr!("inspect-area-arms"),
            Area::Legs => aurora_i18n::tr!("inspect-area-legs"),
            Area::Hud => aurora_i18n::tr!("inspect-area-hud"),
            Area::Other => aurora_i18n::tr!("inspect-area-other"),
            Area::World => aurora_i18n::tr!("inspect-area-world"),
        }
    }
}

/// One inspected object: its root row and the rows of its prims.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub root: Uuid,
    /// Index of the root row in the rows.
    pub first: usize,
    pub prims: Vec<usize>,
    pub point: u8,
    pub area: Area,
    pub complexity: u32,
    pub faces: u32,
    pub vertices: u32,
    pub triangles: u32,
    pub texture_memory: u64,
    pub vram: u64,
}

/// The rows (each linkset root first, its prims after it, as
/// `App::inspect_prims` lists them) gathered by linkset.
pub fn items(rows: &[Row]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let start = row.root || out.is_empty();
        if start {
            out.push(Item {
                root: row.id,
                first: i,
                prims: Vec::new(),
                point: row.point,
                area: Area::of_point(row.point),
                complexity: 0,
                faces: 0,
                vertices: 0,
                triangles: 0,
                texture_memory: 0,
                vram: 0,
            });
        }
        let Some(item) = out.last_mut() else { continue };
        item.prims.push(i);
        item.complexity += row.complexity;
        item.faces += row.faces;
        item.vertices += row.vertices;
        item.triangles += row.triangles;
        item.texture_memory += row.texture_memory;
        item.vram += row.vram;
    }
    out
}

/// The figures of all the items, that an item's share is of.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Whole {
    pub complexity: u64,
    pub faces: u64,
    pub vertices: u64,
    pub triangles: u64,
    pub texture_memory: u64,
    pub vram: u64,
}

impl Whole {
    pub fn of(items: &[Item]) -> Whole {
        items.iter().fold(Whole::default(), |w, i| Whole {
            complexity: w.complexity + u64::from(i.complexity),
            faces: w.faces + u64::from(i.faces),
            vertices: w.vertices + u64::from(i.vertices),
            triangles: w.triangles + u64::from(i.triangles),
            texture_memory: w.texture_memory + i.texture_memory,
            vram: w.vram + i.vram,
        })
    }
}

/// The orders of the View panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Sort {
    /// Head to toe, by section (objects in the world: the linksets' order).
    #[default]
    Area,
    Name,
    Heaviest,
    Newest,
}

/// The items to list for `query` (name or creator, ignoring case), in the
/// order of `sort`.
pub fn shown<'a>(rows: &[Row], items: &'a [Item], query: &str, sort: Sort) -> Vec<&'a Item> {
    let query = query.trim().to_lowercase();
    let mut list: Vec<&Item> = items
        .iter()
        .filter(|item| {
            let row = &rows[item.first];
            query.is_empty() || row.name.to_lowercase().contains(&query) || row.creator_name.to_lowercase().contains(&query)
        })
        .collect();
    let name = |item: &Item| &rows[item.first].name;
    match sort {
        // a stable sort keeps the linksets' order inside a section
        Sort::Area => list.sort_by_key(|item| item.area),
        Sort::Name => list.sort_by(|a, b| crate::natural_order::cmp(name(a), name(b))),
        Sort::Heaviest => list.sort_by_key(|item| std::cmp::Reverse(item.complexity)),
        Sort::Newest => list.sort_by_key(|item| std::cmp::Reverse(rows[item.first].created)),
    }
    list
}

/// The badge of a complexity: light, medium, or among the heaviest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Light,
    Medium,
    Heavy,
}

/// Complexity from which an item is "medium" (white badge).
const MEDIUM: u32 = 3_000;

/// The badge of each of `items`: the two heaviest are Heavy (orange) when
/// there are at least four items to compare, so that a pair of earrings is
/// not marked heavy for being two of two.
pub fn tiers(items: &[Item]) -> Vec<Tier> {
    let mut sorted: Vec<u32> = items.iter().map(|i| i.complexity).collect();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let heavy_from = (items.len() >= 4).then(|| sorted[1]).filter(|v| *v > 0);
    items
        .iter()
        .map(|i| match heavy_from {
            Some(from) if i.complexity >= from => Tier::Heavy,
            _ if i.complexity >= MEDIUM => Tier::Medium,
            _ => Tier::Light,
        })
        .collect()
}

/// Percent of `total` that `part` is, at least 1 when there is a part.
pub fn share(part: u64, total: u64) -> u32 {
    if part == 0 || total == 0 {
        return 0;
    }
    ((part * 100 + total / 2) / total).clamp(1, 100) as u32
}

/// A complexity in a badge: "900", "12.9k", "129k".
pub fn short(value: u32) -> String {
    match value {
        0..=999 => aurora_i18n::integer(i64::from(value)),
        1_000..=9_999 => aurora_i18n::tr!("inspect-thousands", n = aurora_i18n::number(f64::from(value) / 1000.0, 1)),
        _ => aurora_i18n::tr!("inspect-thousands", n = aurora_i18n::integer(i64::from(value / 1000))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u128, root: bool, name: &str, point: u8, complexity: u32) -> Row {
        Row {
            id: Uuid::from_u128(n),
            root,
            name: name.into(),
            creator_name: format!("Maker {n}"),
            point,
            complexity,
            triangles: 100,
            ..Row::default()
        }
    }

    #[test]
    fn attachment_points_fall_in_their_body_area() {
        assert_eq!(Area::of_point(2), Area::Head); // skull
        assert_eq!(Area::of_point(52), Area::Head); // tongue
        assert_eq!(Area::of_point(1), Area::Body); // chest
        assert_eq!(Area::of_point(10), Area::Body); // pelvis
        assert_eq!(Area::of_point(46), Area::Body); // right wing
        assert_eq!(Area::of_point(5), Area::Arms); // left hand
        assert_eq!(Area::of_point(42), Area::Arms); // right ring finger
        assert_eq!(Area::of_point(7), Area::Legs); // left foot
        assert_eq!(Area::of_point(55), Area::Legs); // right hind foot
        assert_eq!(Area::of_point(33), Area::Hud);
        assert_eq!(Area::of_point(0), Area::World);
        assert_eq!(Area::of_point(200), Area::Other);
    }

    #[test]
    fn prims_gather_under_their_linkset_root() {
        let rows = [
            row(1, true, "Head", 2, 900),
            row(2, false, "Teeth", 2, 0),
            row(3, true, "Boots", 7, 400),
            row(4, false, "Lace", 7, 0),
        ];
        let list = items(&rows);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].prims, [0, 1]);
        assert_eq!(list[1].prims, [2, 3]);
        assert_eq!((list[0].triangles, list[0].complexity, list[0].area), (200, 900, Area::Head));
        assert_eq!(list[1].area, Area::Legs);
    }

    #[test]
    fn search_finds_names_and_creators_and_sorts() {
        let rows = [
            row(1, true, "Boots", 7, 400),
            row(2, true, "Aster Head", 2, 900),
            row(3, true, "Hair", 2, 700),
        ];
        let list = items(&rows);
        let names = |q: &str, sort| {
            shown(&rows, &list, q, sort)
                .iter()
                .map(|i| rows[i.first].name.clone())
                .collect::<Vec<_>>()
        };
        // head to toe, the linksets' order kept inside the head
        assert_eq!(names("", Sort::Area), ["Aster Head", "Hair", "Boots"]);
        assert_eq!(names("", Sort::Name), ["Aster Head", "Boots", "Hair"]);
        assert_eq!(names("", Sort::Heaviest), ["Aster Head", "Hair", "Boots"]);
        assert_eq!(names("ASTER", Sort::Area), ["Aster Head"]);
        assert_eq!(names("maker 1", Sort::Area), ["Boots"]);
    }

    #[test]
    fn only_the_two_heaviest_of_four_or_more_are_heavy() {
        let few = items(&[row(1, true, "A", 2, 90_000), row(2, true, "B", 2, 100)]);
        assert_eq!(tiers(&few), [Tier::Medium, Tier::Light]);
        let many = items(&[
            row(1, true, "A", 2, 9_000),
            row(2, true, "B", 2, 21_000),
            row(3, true, "C", 2, 12_000),
            row(4, true, "D", 2, 500),
        ]);
        assert_eq!(tiers(&many), [Tier::Medium, Tier::Heavy, Tier::Heavy, Tier::Light]);
    }

    #[test]
    fn shares_round_and_never_hide_a_small_part() {
        assert_eq!(share(16, 100), 16);
        assert_eq!(share(1, 1_000), 1);
        assert_eq!(share(0, 100), 0);
        assert_eq!(share(5, 0), 0);
        assert_eq!(share(100, 100), 100);
    }
}
