//! Pathfinding lists and edits: RegionObjects, CharacterProperties,
//! ObjectNavMeshProperties and TerrainNavMeshProperties, following
//! LLPathfindingManager (requestGetLinksets, requestSetLinksets,
//! requestGetCharacters, indra/newview/llpathfindingmanager.cpp) and
//! LLPathfinding{Object,Linkset,Character} (indra/newview, originally
//! LGPL 2.1).
//!
//! No Havok: the navmesh source (RetrieveNavMeshSrc) is a blob only Linden's
//! licensed Havok library reads; Aurora never requests it (`caps.rs` test).
pub mod linkset;
pub mod navmesh;

use aurora_llsd::Llsd;
use linkset::{Category, Linkset, Use};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Objects,
    Characters,
}

impl Kind {
    pub fn capability(self) -> &'static str {
        match self {
            Self::Objects => "RegionObjects",
            Self::Characters => "CharacterProperties",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The region has no such capability (kRequestNotEnabled).
    Unsupported,
    Failed,
    InvalidReply,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Detail {
    Object {
        land_impact: u32,
        /// `is_scripted`; None when the region doesn't say (the Legacy
        /// Viewer's "unknown").
        scripted: Option<bool>,
        linkset: Linkset,
    },
    Character {
        cpu_us: f64,
        horizontal: bool,
        length: f64,
        radius: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Nil for the terrain row.
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: Uuid,
    pub group_owned: bool,
    pub position: [f64; 3],
    pub detail: Detail,
}

impl Row {
    pub fn linkset(&self) -> Option<&Linkset> {
        match &self.detail {
            Detail::Object { linkset, .. } => Some(linkset),
            Detail::Character { .. } => None,
        }
    }
}

const MAX_ROWS: usize = 100_000;

fn number(value: &Llsd) -> Option<f64> {
    let n = match value {
        Llsd::Real(n) => *n,
        Llsd::Integer(n) => f64::from(*n),
        _ => return None,
    };
    n.is_finite().then_some(n)
}

fn text(value: &Llsd) -> Option<String> {
    match value {
        Llsd::String(s) if s.len() <= 4096 => Some(s.clone()),
        _ => None,
    }
}

/// parsePathfindingData: the use and the coefficients, shared by objects
/// and the terrain. `can_be_volume` defaults to true for an object and
/// false for the terrain, as the two LLPathfindingLinkset constructors do.
fn linkset(value: &Llsd, terrain: bool) -> Option<Linkset> {
    let category = match value["navmesh_category"] {
        Llsd::Integer(n) => Category::from_wire(n)?,
        _ => return None,
    };
    let mut walkability = [0; 4];
    for (slot, field) in walkability.iter_mut().zip(["A", "B", "C", "D"]) {
        *slot = match value[field] {
            Llsd::Integer(n @ 0..=100) => n as u8,
            _ => return None,
        };
    }
    let can_be_volume = match &value["can_be_volume"] {
        Llsd::Boolean(b) => *b,
        _ => !terrain,
    };
    Some(Linkset {
        use_: Use::new(value["phantom"].as_bool(), category),
        walkability,
        modifiable: !terrain && value["modifiable"].as_bool(),
        can_be_volume,
        terrain,
    })
}

fn row(key: &str, value: &Llsd, kind: Kind) -> Option<Row> {
    let id = Uuid::parse_str(key).ok().filter(|id| !id.is_nil())?;
    let position = value["position"].as_array();
    if position.len() != 3 {
        return None;
    }
    let position = [number(&position[0])?, number(&position[1])?, number(&position[2])?];
    let owner = match value["owner"] {
        Llsd::Uuid(id) => id,
        _ => return None,
    };
    let detail = match kind {
        Kind::Objects => Detail::Object {
            land_impact: match value["landimpact"] {
                Llsd::Integer(n) if n >= 0 => n as u32,
                _ => return None,
            },
            scripted: match &value["is_scripted"] {
                Llsd::Boolean(b) => Some(*b),
                _ => None,
            },
            linkset: linkset(value, false)?,
        },
        Kind::Characters => {
            let cpu_us = number(&value["cpu_time"])?;
            let length = number(&value["length"])?;
            let radius = number(&value["radius"])?;
            if cpu_us < 0.0 || length < 0.0 || radius < 0.0 {
                return None;
            }
            Detail::Character {
                cpu_us,
                length,
                radius,
                horizontal: value["horizontal"].as_bool(),
            }
        }
    };
    Some(Row {
        id,
        name: text(&value["name"])?,
        description: text(&value["description"])?,
        owner,
        group_owned: value["owner_is_group"].as_bool(),
        position,
        detail,
    })
}

/// A RegionObjects / CharacterProperties reply, or an
/// ObjectNavMeshProperties answer (the linksets it changed): a map of
/// `{ <id>: { … } }`. Empty entries are skipped (parseLinksetListData). A
/// malformed entry is skipped and logged, not a failure of the whole list:
/// the Legacy Viewer only asserts on it, and a region can hold thousands.
/// A region with nothing to list may answer with an empty body: an empty
/// list, as the Legacy Viewer's beginMap() iterates nothing.
pub fn parse(data: &Llsd, kind: Kind) -> Result<Vec<Row>, Error> {
    match data {
        Llsd::Undef => return Ok(Vec::new()),
        Llsd::Array(items) if items.is_empty() => return Ok(Vec::new()),
        Llsd::String(text) if text.trim().is_empty() => return Ok(Vec::new()),
        _ => {}
    }
    let Some(map) = data.as_map().filter(|map| map.len() <= MAX_ROWS) else {
        let shape = data.as_map().map_or_else(
            || format!("{data:?}").chars().take(80).collect::<String>(),
            |map| format!("{} entries", map.len()),
        );
        log::warn!("pathfinding: {} reply not understood: {shape}", kind.capability());
        return Err(Error::InvalidReply);
    };
    let mut rows = Vec::with_capacity(map.len());
    let mut skipped = 0usize;
    for (key, value) in map {
        if value.is_undef() || value.as_map().is_some_and(|m| m.is_empty()) {
            continue;
        }
        match row(key, value, kind) {
            Some(row) => rows.push(row),
            None => skipped += 1,
        }
    }
    if skipped > 0 {
        log::warn!("pathfinding: {skipped} malformed {} entries skipped", kind.capability());
    }
    rows.sort_by_cached_key(|row| (row.name.to_lowercase(), row.id));
    Ok(rows)
}

/// A TerrainNavMeshProperties reply (GET or PUT): the terrain as one
/// linkset (LLPathfindingLinkset's terrain constructor), as a row with a
/// nil id and no name; the windows show their terrain strings for it.
pub fn parse_terrain(data: &Llsd) -> Result<Row, Error> {
    let linkset = linkset(data, true).ok_or(Error::InvalidReply)?;
    Ok(Row {
        id: Uuid::nil(),
        name: String::new(),
        description: String::new(),
        owner: Uuid::nil(),
        group_owned: false,
        position: [0.0; 3],
        detail: Detail::Object {
            land_impact: 0,
            scripted: Some(false),
            linkset,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn entry() -> Llsd {
        llsd_map! { "name" => "Test", "description" => "Description", "owner" => Uuid::from_u128(7),
        "position" => Llsd::Array(vec![1.into(), 2.into(), 3.into()]), "landimpact" => 12,
        "navmesh_category" => 1, "A" => 0, "B" => 25, "C" => 75, "D" => 100, "phantom" => true,
        "modifiable" => true, "cpu_time" => 123.0, "length" => 2.0, "radius" => 0.5, "horizontal" => false }
    }

    #[test]
    fn reference_maps_skip_empty_entries_and_preserve_real_fields() {
        let mut data = Llsd::new_map();
        let id = Uuid::from_u128(9);
        data.insert(id.to_string(), entry());
        data.insert(Uuid::from_u128(10).to_string(), Llsd::Undef);
        let objects = parse(&data, Kind::Objects).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, id);
        assert_eq!(
            objects[0].detail,
            Detail::Object {
                land_impact: 12,
                scripted: None,
                linkset: Linkset {
                    use_: Use::ExclusionVolume,
                    walkability: [0, 25, 75, 100],
                    modifiable: true,
                    can_be_volume: true,
                    terrain: false,
                },
            }
        );
        assert!(matches!(
            parse(&data, Kind::Characters).unwrap()[0].detail,
            Detail::Character {
                cpu_us: 123.0,
                radius: 0.5,
                ..
            }
        ));
    }

    #[test]
    fn malformed_entries_are_skipped_without_losing_the_list() {
        let good = Uuid::from_u128(1);
        for field in ["id", "position", "A", "navmesh_category", "name"] {
            let mut value = entry();
            match field {
                "position" => value.insert(field, Llsd::Array(vec![f64::NAN.into(), 0.into(), 0.into()])),
                "A" => value.insert(field, 101),
                "navmesh_category" => value.insert(field, 3),
                "name" => value.insert(field, 5),
                _ => {}
            }
            let mut data = Llsd::new_map();
            data.insert(good.to_string(), entry());
            data.insert(
                if field == "id" {
                    "bad".into()
                } else {
                    Uuid::from_u128(9).to_string()
                },
                value,
            );
            let rows = parse(&data, Kind::Objects).unwrap();
            assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [good], "{field}");
        }
        assert_eq!(parse(&Llsd::Integer(1), Kind::Objects), Err(Error::InvalidReply));
        // a region with nothing to list may send an empty body
        assert_eq!(parse(&Llsd::Undef, Kind::Characters), Ok(Vec::new()));
        assert_eq!(parse(&Llsd::Array(Vec::new()), Kind::Characters), Ok(Vec::new()));
    }

    #[test]
    fn scripted_and_can_be_volume_are_read_when_given() {
        let mut value = entry();
        value.insert("is_scripted", true);
        value.insert("can_be_volume", false);
        let mut data = Llsd::new_map();
        data.insert(Uuid::from_u128(4).to_string(), value);
        let rows = parse(&data, Kind::Objects).unwrap();
        let Detail::Object { scripted, linkset, .. } = &rows[0].detail else {
            panic!("an object");
        };
        assert_eq!(*scripted, Some(true));
        assert!(!linkset.can_be_volume);
    }

    #[test]
    fn the_terrain_is_one_unmodifiable_non_volume_linkset() {
        let body = llsd_map! { "navmesh_category" => 0, "A" => 100, "B" => 90, "C" => 80, "D" => 70,
        "modifiable" => true };
        let row = parse_terrain(&body).unwrap();
        assert!(row.id.is_nil());
        let linkset = row.linkset().copied().expect("a linkset");
        assert_eq!(linkset.use_, Use::Walkable);
        assert_eq!(linkset.walkability, [100, 90, 80, 70]);
        assert!(linkset.terrain && !linkset.modifiable && !linkset.can_be_volume);
        assert_eq!(parse_terrain(&llsd_map! { "A" => 1 }), Err(Error::InvalidReply));
    }
}
