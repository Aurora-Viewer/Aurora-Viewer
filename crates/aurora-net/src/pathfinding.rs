//! Read-only RegionObjects / CharacterProperties, following
//! LLPathfindingManager::requestGetLinksets/requestGetCharacters and
//! LLPathfinding{Object,Linkset,Character} (indra/newview, originally LGPL 2.1).
use aurora_llsd::Llsd;
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
    Unsupported,
    Failed,
    InvalidReply,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Detail {
    Object {
        land_impact: u32,
        category: u8,
        phantom: bool,
        walkability: [u8; 4],
        modifiable: bool,
        scripted: bool,
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
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: Uuid,
    pub group_owned: bool,
    pub position: [f64; 3],
    pub detail: Detail,
}

fn number(value: &Llsd) -> Result<f64, Error> {
    let n = match value {
        Llsd::Real(n) => *n,
        Llsd::Integer(n) => f64::from(*n),
        _ => return Err(Error::InvalidReply),
    };
    if n.is_finite() { Ok(n) } else { Err(Error::InvalidReply) }
}

fn text(value: &Llsd) -> Result<String, Error> {
    match value {
        Llsd::String(s) if s.len() <= 4096 => Ok(s.clone()),
        _ => Err(Error::InvalidReply),
    }
}

pub fn parse(data: &Llsd, kind: Kind) -> Result<Vec<Row>, Error> {
    let map = data.as_map().filter(|map| map.len() <= 100_000).ok_or(Error::InvalidReply)?;
    let mut rows = Vec::with_capacity(map.len());
    for (key, value) in map {
        if value.is_undef() || value.as_map().is_some_and(|m| m.is_empty()) {
            continue;
        }
        let id = Uuid::parse_str(key).ok().filter(|id| !id.is_nil()).ok_or(Error::InvalidReply)?;
        let position = value["position"].as_array();
        if position.len() != 3 {
            return Err(Error::InvalidReply);
        }
        let position = [number(&position[0])?, number(&position[1])?, number(&position[2])?];
        let owner = match value["owner"] {
            Llsd::Uuid(id) => id,
            _ => return Err(Error::InvalidReply),
        };
        let detail = match kind {
            Kind::Objects => {
                let land_impact = match value["landimpact"] {
                    Llsd::Integer(n) if n >= 0 => n as u32,
                    _ => return Err(Error::InvalidReply),
                };
                // Wire constants: Include=0, Exclude=1, Ignore=2.
                let category = match value["navmesh_category"] {
                    Llsd::Integer(n @ 0..=2) => n as u8,
                    _ => return Err(Error::InvalidReply),
                };
                let mut walkability = [0; 4];
                for (i, field) in ["A", "B", "C", "D"].iter().enumerate() {
                    walkability[i] = match value[*field] {
                        Llsd::Integer(n @ 0..=100) => n as u8,
                        _ => return Err(Error::InvalidReply),
                    };
                }
                Detail::Object {
                    land_impact,
                    category,
                    walkability,
                    phantom: value["phantom"].as_bool(),
                    modifiable: value["modifiable"].as_bool(),
                    scripted: value["is_scripted"].as_bool(),
                }
            }
            Kind::Characters => {
                let cpu_us = number(&value["cpu_time"])?;
                let length = number(&value["length"])?;
                let radius = number(&value["radius"])?;
                if cpu_us < 0.0 || length < 0.0 || radius < 0.0 {
                    return Err(Error::InvalidReply);
                }
                Detail::Character {
                    cpu_us,
                    length,
                    radius,
                    horizontal: value["horizontal"].as_bool(),
                }
            }
        };
        rows.push(Row {
            id,
            name: text(&value["name"])?,
            description: text(&value["description"])?,
            owner,
            group_owned: value["owner_is_group"].as_bool(),
            position,
            detail,
        });
    }
    rows.sort_by_cached_key(|row| (row.name.to_lowercase(), row.id));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn row() -> Llsd {
        llsd_map! { "name" => "Test", "description" => "Description", "owner" => Uuid::from_u128(7),
        "position" => Llsd::Array(vec![1.into(), 2.into(), 3.into()]), "landimpact" => 12,
        "navmesh_category" => 1, "A" => 0, "B" => 25, "C" => 75, "D" => 100, "phantom" => true,
        "cpu_time" => 123.0, "length" => 2.0, "radius" => 0.5, "horizontal" => false }
    }

    #[test]
    fn reference_maps_skip_empty_entries_and_preserve_real_fields() {
        let mut data = Llsd::new_map();
        let id = Uuid::from_u128(9);
        data.insert(id.to_string(), row());
        data.insert(Uuid::from_u128(10).to_string(), Llsd::Undef);
        let objects = parse(&data, Kind::Objects).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, id);
        assert!(matches!(
            objects[0].detail,
            Detail::Object {
                category: 1,
                phantom: true,
                walkability: [0, 25, 75, 100],
                ..
            }
        ));
        assert!(matches!(
            parse(&data, Kind::Characters).unwrap()[0].detail,
            Detail::Character {
                cpu_us: 123.0,
                radius: 0.5,
                ..
            }
        ));
        data.insert(id.to_string(), llsd_map! { "name" => "Incomplete" });
        assert_eq!(parse(&data, Kind::Objects), Err(Error::InvalidReply));
    }

    #[test]
    fn malformed_ids_nonfinite_coordinates_and_out_of_range_walkability_are_rejected() {
        for field in ["id", "position", "A", "navmesh_category"] {
            let mut value = row();
            match field {
                "position" => value.insert(field, Llsd::Array(vec![f64::NAN.into(), 0.into(), 0.into()])),
                "A" => value.insert(field, 101),
                "navmesh_category" => value.insert(field, 3),
                _ => {}
            }
            let mut data = Llsd::new_map();
            data.insert(
                if field == "id" {
                    "bad".into()
                } else {
                    Uuid::from_u128(9).to_string()
                },
                value,
            );
            assert_eq!(parse(&data, Kind::Objects), Err(Error::InvalidReply));
        }
    }
}
