//! Directory of the demo (`AURORA_DEMO_SEARCH`): the answers of a simulator
//! to the queries of the search window, so each tab can be tried offline.
//! People, groups, places, land for sale, events and classifieds are a few
//! rows around the demo's residents, groups and parcels; a text containing
//! "page" answers a full page and one more row (the next page button), a
//! text containing "xyz" answers nothing. EventInfoRequest,
//! GroupProfileRequest, ClassifiedInfoRequest and ProductInfoRequest are
//! answered for these rows; avatar profiles and parcels by the demo's usual
//! replies.

use super::{DEMO_GROUP1, DEMO_GROUP2, DEMO_LOUP, DEMO_NOVA, TEX_SNAPSHOT};
use aurora_net::search::{
    ClassifiedRow, DirQuery, DirReply, DirRows, EventInfo, EventRow, GroupProfile, GroupRow, LandRow, PersonRow, PlaceRow, dfq,
    event_flags, status,
};
use aurora_net::{NetCommand, NetEvent};
use glam::DVec3;
use uuid::Uuid;

/// Ids of the demo classifieds and of the residents invented for the list.
const CLASSIFIED: u128 = 0x5EA2_C1A5_0000_0000_0000_0000_0000_0000;
const RESIDENT: u128 = 0x5EA2_A6E1_0000_0000_0000_0000_0000_0000;
const GROUP: u128 = 0x5EA2_6E00_0000_0000_0000_0000_0000_0000;
/// First demo event; their dates start at this Unix time.
const EVENT: u32 = 5001;
const EVENT_TIME: u32 = 1_791_475_200;

fn id(base: u128, n: usize) -> Uuid {
    Uuid::from_u128(base | n as u128)
}

/// The demo group the agent is not in, with an open enrollment and a fee
/// ("Boreal Photographers"): the guest's view of the group profile.
pub fn boreal() -> Uuid {
    id(GROUP, 4)
}

/// The member count a demo group's profile gives.
pub fn member_count(group: &Uuid) -> Option<i32> {
    groups().into_iter().find(|g| g.0 == *group).map(|g| g.2)
}

/// Its founder, as in the profile.
pub fn founder(group: &Uuid) -> Option<Uuid> {
    let i = groups().iter().position(|g| g.0 == *group)?;
    Some(if i % 2 == 0 { DEMO_LOUP } else { DEMO_NOVA })
}

/// A demo group has this name already (the dataserver's check of a new
/// group's name; case ignored).
pub fn group_named(name: &str) -> bool {
    groups().iter().any(|g| g.1.eq_ignore_ascii_case(name.trim()))
}

/// (name, members) of the demo groups; the first two are the demo's own.
fn groups() -> Vec<(Uuid, String, i32)> {
    vec![
        (DEMO_GROUP1, "Aurora Builders".to_owned(), 1248),
        (DEMO_GROUP2, aurora_i18n::tr!("demo-northern-wolves").to_owned(), 86),
        (id(GROUP, 3), "Nordheim Sailing Club".to_owned(), 412),
        (id(GROUP, 4), "Boreal Photographers".to_owned(), 2731),
        (id(GROUP, 5), "Aurora Viewer Testers".to_owned(), 19),
        (id(GROUP, 6), "Pixel Boutique VIP".to_owned(), 5560),
    ]
}

/// (name, hour offset, flags) of the demo events.
const EVENTS: [(&str, u32, u32); 6] = [
    ("Aurora Jazz Night", 0, event_flags::NONE),
    ("Nordheim Regatta", 2, event_flags::NONE),
    ("Boreal Photo Walk", 5, event_flags::NONE),
    ("Pixel Boutique Fashion Show", 8, event_flags::MATURE),
    ("Midnight Viking Feast", 12, event_flags::ADULT),
    ("Building Class: Mesh Basics", 26, event_flags::NONE),
];

/// (name, listing price) of the demo classifieds.
fn classifieds() -> Vec<(String, i32)> {
    vec![
        (aurora_i18n::tr!("demo-wolf-s-workshop-mesh-furniture").to_owned(), 50),
        ("Pixel Boutique".to_owned(), 1200),
        ("Nordheim Rentals".to_owned(), 300),
        ("Boreal Lagoon Club".to_owned(), 75),
        ("Aurora Skyboxes".to_owned(), 150),
    ]
}

/// "MM/DD HH:MM" in Pacific time, as the directory writes event dates.
fn event_date(unix: u32) -> String {
    let (_, month, day, _, hour, minute, _) = crate::ui::land::slt_parts(unix as i64);
    format!("{month:02}/{day:02} {hour:02}:{minute:02}")
}

fn people(text: &str, start: i32) -> Vec<PersonRow> {
    // the names the demo's name service gives to these ids, first
    let known = [
        ("Nova", aurora_i18n::tr!("demo-example")),
        ("pixelboutique", "Resident"),
        ("Orion", aurora_i18n::tr!("demo-example")),
        ("tesstouch", "Resident"),
    ];
    let invented = ["Aria", "Bjorn", "Cléo", "Dario", "Elsa", "Faye", "Gaël", "Hana", "Ivo", "Juno"];
    let count = match (text.contains("page"), start) {
        (true, 0) => aurora_net::search::RESULT_PAGE_SIZE + 1,
        (true, _) => 41,
        _ => 14,
    };
    (0..count)
        .map(|i| {
            let n = start as usize + i;
            let (first, last) = match known.get(n) {
                Some((first, last)) => ((*first).to_owned(), (*last).to_owned()),
                None => (format!("{}{:02}", invented[n % invented.len()], n), "Resident".to_owned()),
            };
            PersonRow {
                id: id(RESIDENT, n),
                first_name: first,
                last_name: last,
                online: n.is_multiple_of(3),
            }
        })
        .collect()
}

fn events() -> Vec<EventRow> {
    EVENTS
        .iter()
        .enumerate()
        .map(|(i, (name, hours, flags))| {
            let unix = EVENT_TIME + hours * 3600;
            EventRow {
                owner: if i % 2 == 0 { DEMO_LOUP } else { DEMO_NOVA },
                name: (*name).to_owned(),
                id: EVENT + i as u32,
                date: event_date(unix),
                unix_time: unix,
                flags: *flags,
            }
        })
        .collect()
}

fn query_reply(query_id: Uuid, start: i32, query: &DirQuery) -> DirReply {
    let nothing = |text: &str| text.contains("xyz");
    let (status, rows) = match query {
        DirQuery::Find { text, flags } if flags & dfq::PEOPLE != 0 => {
            let rows = if nothing(text) { Vec::new() } else { people(text, start) };
            (None, DirRows::People(rows))
        }
        DirQuery::Find { text, flags } if flags & dfq::GROUPS != 0 => {
            let rows = groups()
                .into_iter()
                .filter(|_| !nothing(text))
                .map(|(id, name, members)| GroupRow {
                    id,
                    name,
                    members,
                    search_order: 0.0,
                })
                .collect();
            (None, DirRows::Groups(rows))
        }
        DirQuery::Find { text, .. } => {
            if nothing(text) {
                (Some(status::FOUND_NONE), DirRows::Events(Vec::new()))
            } else {
                (None, DirRows::Events(events()))
            }
        }
        DirQuery::Places { text, .. } => {
            if nothing(text) {
                (Some(status::FOUND_NONE), DirRows::Places(Vec::new()))
            } else {
                let rows = super::place::directory()
                    .into_iter()
                    .map(|(parcel, name, dwell, _)| PlaceRow {
                        parcel,
                        name,
                        for_sale: false,
                        auction: false,
                        dwell,
                    })
                    .collect();
                (None, DirRows::Places(rows))
            }
        }
        DirQuery::Land { .. } => {
            let rows = super::place::directory()
                .into_iter()
                .enumerate()
                // the first parcel is for sale, the others are auctions (listed
                // at 0, what Firestorm's default price limit lets through)
                .map(|(i, (parcel, name, _, area))| LandRow {
                    parcel,
                    name,
                    auction: i != 0,
                    for_sale: i == 0,
                    price: if i == 0 { area * 3 / 2 } else { 0 },
                    area,
                    sku: if i % 2 == 0 { "023" } else { "029" }.to_owned(),
                })
                .collect();
            (None, DirRows::Land(rows))
        }
        DirQuery::Classifieds { text, .. } => {
            if nothing(text) {
                (Some(status::FOUND_NONE), DirRows::Classifieds(Vec::new()))
            } else {
                let rows = classifieds()
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, price))| ClassifiedRow {
                        id: id(CLASSIFIED, i + 1),
                        name,
                        flags: 0,
                        creation_date: 1_759_000_000,
                        expiration_date: 1_790_000_000,
                        price,
                    })
                    .collect();
                (None, DirRows::Classifieds(rows))
            }
        }
    };
    DirReply { query_id, status, rows }
}

/// The answers of the directory; None when `cmd` is not one of its requests.
pub fn reply(cmd: &NetCommand) -> Option<Vec<NetEvent>> {
    let position = DVec3::new(256_000.0 + 128.0, 256_000.0 + 128.0, 30.0);
    Some(match cmd {
        NetCommand::DirQuery { id, start, query } => vec![NetEvent::DirReply(Box::new(query_reply(*id, *start, query)))],
        NetCommand::EventInfoRequest(event) => {
            let i = event.checked_sub(EVENT).map(|i| i as usize).filter(|i| *i < EVENTS.len())?;
            let (name, hours, flags) = EVENTS[i];
            let unix = EVENT_TIME + hours * 3600;
            vec![NetEvent::EventInfo(Box::new(EventInfo {
                id: *event,
                creator: if i % 2 == 0 { DEMO_LOUP } else { DEMO_NOVA },
                name: name.to_owned(),
                category: [
                    "Live Music",
                    "Sports",
                    "Arts and Culture",
                    "Commercial",
                    "Nightlife/Entertainment",
                    "Education",
                ][i]
                    .to_owned(),
                desc: aurora_i18n::tr!("demo-a-quiet-spot-by-the-water-perfect-for-watching").to_owned(),
                date: event_date(unix),
                date_utc: unix,
                duration: 60 + 30 * i as u32,
                cover: (i % 3 == 1).then_some(50),
                sim_name: aurora_i18n::tr!("common-aurora-demo").to_owned(),
                global: position + DVec3::new(12.0 * i as f64, -8.0 * i as f64, 0.0),
                flags,
            }))]
        }
        NetCommand::GroupProfileRequest(group) => {
            let (i, (_, name, members)) = groups().into_iter().enumerate().find(|(_, g)| g.0 == *group)?;
            vec![NetEvent::GroupProfile(Box::new(GroupProfile {
                id: *group,
                name,
                charter: aurora_i18n::tr!("demo-resident-in-aurora-viewer-s-offline-demo").to_owned(),
                show_in_list: true,
                member_title: if i < 2 { "Member".to_owned() } else { String::new() },
                insignia: TEX_SNAPSHOT,
                founder: if i % 2 == 0 { DEMO_LOUP } else { DEMO_NOVA },
                membership_fee: if i == 3 { 250 } else { 0 },
                open_enrollment: i != 1,
                members,
                roles: 3,
                allow_publish: true,
                mature_publish: i == 5,
                ..Default::default()
            }))]
        }
        NetCommand::ClassifiedInfoRequest(classified) => {
            let n = classified.as_u128() ^ CLASSIFIED;
            let (name, price) = classifieds().into_iter().nth((n as usize).checked_sub(1)?)?;
            vec![NetEvent::ClassifiedInfo(Box::new(aurora_net::ClassifiedInfo {
                id: *classified,
                creator: DEMO_LOUP,
                creation_date: 1_759_000_000,
                expiration_date: 1_790_000_000,
                category: 1 + (n as u32 - 1) % 4,
                name,
                desc: aurora_i18n::tr!("demo-lightweight-mesh-furniture-copyable-and-modifiable-come-visit-the").to_owned(),
                snapshot: TEX_SNAPSHOT,
                sim_name: aurora_i18n::tr!("common-aurora-demo").to_owned(),
                parcel_name: aurora_i18n::tr!("common-aurora-plaza").to_owned(),
                pos_global: position,
                flags: 0,
                price,
                ..Default::default()
            }))]
        }
        NetCommand::ProductInfoRequest => vec![NetEvent::ProductInfo(vec![
            ("023".to_owned(), "Mainland / Full Region".to_owned()),
            ("029".to_owned(), "Estate / Homestead".to_owned()),
        ])],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::search::{Key, Request, Status, Tab};
    use aurora_net::search::{AgentAccess, EventsMode, LandFilter, Ratings, SIM_ACCESS_ADULT, SIM_ACCESS_PG};
    use std::sync::Arc;
    use std::time::Instant;

    fn world() -> crate::world::World {
        crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    /// Run the commands of the model through the demo directory until
    /// nothing is left to ask.
    fn pump(world: &mut crate::world::World) -> Vec<NetCommand> {
        let mut sent = Vec::new();
        for _ in 0..8 {
            let cmds = world.search.take_commands();
            if cmds.is_empty() {
                break;
            }
            for c in cmds {
                for ev in super::super::demo_reply(&c) {
                    world.apply(ev);
                }
                sent.push(c);
            }
        }
        sent
    }

    #[test]
    fn every_tab_gets_rows_and_details() {
        let mut world = world();
        let access = AgentAccess {
            max: SIM_ACCESS_ADULT,
            preferred: SIM_ACCESS_ADULT,
        };
        let all = Ratings {
            pg: true,
            mature: true,
            adult: true,
        };
        let now = Instant::now();
        let requests = [
            Request::People { text: "nova".into() },
            Request::Groups {
                text: "aurora".into(),
                maturity: SIM_ACCESS_PG,
            },
            Request::Places {
                text: "aurora".into(),
                ratings: all,
                category: -1,
            },
            Request::Land {
                ratings: all,
                filter: LandFilter {
                    limit_price: false,
                    ..Default::default()
                },
            },
            Request::Events {
                text: String::new(),
                ratings: all,
                mode: EventsMode::Current,
                category: 0,
            },
            Request::Classifieds {
                text: "aurora".into(),
                ratings: all,
                category: 0,
            },
        ];
        for request in requests {
            let tab = request.tab();
            world.search.find(request, access, now);
            let sent = pump(&mut world);
            let list = world.search.list(tab).expect("list");
            assert_eq!(list.status, Status::Found, "{tab:?}");
            assert!(list.rows.len() >= 3, "{tab:?}");
            let selected = list.selected.expect("first row selected");
            // the details of the first row were asked and answered
            assert!(sent.len() >= 2, "{tab:?}: query and details");
            match (tab, selected) {
                (Tab::People, Key::Id(id)) => assert!(world.profiles.get(&id).is_some()),
                (Tab::Groups, Key::Id(id)) => assert!(world.search.groups.contains_key(&id)),
                (Tab::Places | Tab::Land, Key::Id(id)) => assert!(world.profiles.parcels.contains_key(&id)),
                (Tab::Classifieds, Key::Id(id)) => assert!(world.profiles.classifieds.contains_key(&id)),
                (Tab::Events, Key::Event(id)) => assert!(world.search.events.contains_key(&id)),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(world.search.land_types.len(), 2);
        // every demo event has its details
        for i in 0..EVENTS.len() as u32 {
            assert!(reply(&NetCommand::EventInfoRequest(EVENT + i)).is_some());
        }
        assert!(reply(&NetCommand::EventInfoRequest(1)).is_none());
    }

    #[test]
    fn paging_and_nothing_found() {
        let mut world = world();
        let access = AgentAccess::default();
        let now = Instant::now();
        world.search.find(Request::People { text: "page".into() }, access, now);
        pump(&mut world);
        let list = world.search.list(Tab::People).expect("list");
        assert_eq!((list.rows.len(), list.has_next), (100, true));
        world.search.turn_page(Tab::People, true);
        pump(&mut world);
        let list = world.search.list(Tab::People).expect("list");
        assert_eq!((list.rows.len(), list.has_next, list.has_back()), (41, false, true));
        world.search.find(Request::People { text: "xyz".into() }, access, now);
        pump(&mut world);
        assert_eq!(world.search.list(Tab::People).expect("list").status, Status::NotFound("xyz".into()));
        let places = Request::Places {
            text: "xyzzy".into(),
            ratings: Ratings::default(),
            category: -1,
        };
        world.search.find(places, access, now);
        pump(&mut world);
        assert_eq!(
            world.search.list(Tab::Places).expect("list").status,
            Status::NotFound("xyzzy".into())
        );
    }
}
