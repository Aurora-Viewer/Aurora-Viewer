//! The classifieds of the demo residents and ours (AURORA_DEMO_PROFILE):
//! the simulator's side of avatarclassifiedsrequest, ClassifiedInfoRequest,
//! ClassifiedInfoUpdate, ClassifiedDelete and the click statistics. Ours are
//! kept for the run; a new one is published as on the grid, its price taken
//! from the balance by the viewer's demo loop (`price_of_new`).

use super::{DEMO_AGENT, DEMO_FRIEND, DEMO_LOUP, demo_snapshot};
use aurora_net::profile::{ClassifiedStats, classified_flags};
use aurora_net::{ClassifiedInfo, NetCommand, NetEvent};
use std::sync::Mutex;
use uuid::Uuid;

/// The wolf's classified (the classic window's scenario).
pub const WOLF_WORKSHOP: Uuid = Uuid::from_u128(0xC1A5_0001);

static STORE: Mutex<Option<Vec<ClassifiedInfo>>> = Mutex::new(None);

fn seed() -> Vec<ClassifiedInfo> {
    let at = |x: f64, y: f64, z: f64| glam::DVec3::new(256_000.0 + x, 256_000.0 + y, z);
    vec![
        ClassifiedInfo {
            id: WOLF_WORKSHOP,
            creator: DEMO_LOUP,
            creation_date: 1_759_000_000,
            category: 2,
            name: aurora_i18n::tr!("demo-wolf-s-workshop-mesh-furniture").into(),
            desc: aurora_i18n::tr!("demo-lightweight-mesh-furniture-copyable-and-modifiable-come-visit-the").into(),
            sim_name: aurora_i18n::tr!("common-aurora-demo").into(),
            parcel_name: aurora_i18n::tr!("common-aurora-plaza").into(),
            pos_global: at(128.0, 128.0, 30.0),
            flags: classified_flags::AUTO_RENEW,
            price: 50,
            snapshot: demo_snapshot(4),
            ..Default::default()
        },
        ClassifiedInfo {
            id: Uuid::from_u128(0xC1A5_0002),
            creator: DEMO_FRIEND,
            creation_date: 1_759_400_000,
            category: 4,
            name: aurora_i18n::tr!("demo-lagoon-concerts").into(),
            desc: aurora_i18n::tr!("demo-lagoon-concerts-desc").into(),
            sim_name: aurora_i18n::tr!("common-boreal-lagoon").into(),
            parcel_name: aurora_i18n::tr!("demo-fireflies-club").into(),
            pos_global: at(128.0, 64.0, 22.0),
            flags: classified_flags::AUTO_RENEW | classified_flags::QUERY_INC_MATURE,
            price: 120,
            snapshot: demo_snapshot(5),
            ..Default::default()
        },
        ClassifiedInfo {
            id: Uuid::from_u128(0xC1A5_0003),
            creator: DEMO_FRIEND,
            creation_date: 1_759_900_000,
            category: 3,
            name: aurora_i18n::tr!("demo-chalets-for-rent").into(),
            desc: aurora_i18n::tr!("demo-chalets-for-rent-desc").into(),
            sim_name: aurora_i18n::tr!("common-aurora-demo").into(),
            parcel_name: aurora_i18n::tr!("common-aurora-plaza").into(),
            pos_global: at(40.0, 210.0, 58.0),
            flags: 0,
            price: 50,
            snapshot: demo_snapshot(6),
            ..Default::default()
        },
        ClassifiedInfo {
            id: Uuid::from_u128(0xC1A5_0004),
            creator: DEMO_AGENT,
            creation_date: 1_760_000_000,
            category: 4,
            name: aurora_i18n::tr!("demo-building-class").into(),
            desc: aurora_i18n::tr!("demo-building-class-desc").into(),
            sim_name: aurora_i18n::tr!("common-sandbox").into(),
            pos_global: at(60.0, 200.0, 31.0),
            flags: classified_flags::AUTO_RENEW,
            price: 50,
            snapshot: demo_snapshot(4),
            ..Default::default()
        },
    ]
}

fn with<R>(f: impl FnOnce(&mut Vec<ClassifiedInfo>) -> R) -> R {
    let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(store.get_or_insert_with(seed))
}

/// What a ClassifiedInfoUpdate costs: the price of a classified the store
/// does not know yet (an edit costs nothing more).
pub fn price_of_new(cmd: &NetCommand) -> Option<i32> {
    let NetCommand::ClassifiedUpdate(c) = cmd else {
        return None;
    };
    with(|s| s.iter().all(|k| k.id != c.id)).then_some(c.price)
}

fn list_of(store: &[ClassifiedInfo], target: Uuid) -> NetEvent {
    NetEvent::AvatarClassifieds {
        target,
        list: store
            .iter()
            .filter(|c| c.creator == target)
            .map(|c| (c.id, c.name.clone()))
            .collect(),
    }
}

/// The simulator's answer to a classified command, None for the others.
pub fn reply(cmd: &NetCommand) -> Option<Vec<NetEvent>> {
    match cmd {
        NetCommand::ClassifiedsRequest(target) => Some(with(|s| vec![list_of(s, *target)])),
        NetCommand::ClassifiedInfoRequest(id) => Some(with(|s| {
            s.iter()
                .find(|c| c.id == *id)
                .map(|c| vec![NetEvent::ClassifiedInfo(Box::new(c.clone()))])
                .unwrap_or_default()
        })),
        NetCommand::ClassifiedUpdate(c) => Some(with(|s| {
            let mut saved = (**c).clone();
            saved.creator = DEMO_AGENT;
            match s.iter_mut().find(|k| k.id == c.id) {
                Some(k) => {
                    // the creation date and the price stay those of the publication
                    saved.creation_date = k.creation_date;
                    saved.price = k.price;
                    *k = saved.clone();
                }
                None => {
                    saved.creation_date = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as u32);
                    s.push(saved.clone());
                }
            }
            vec![list_of(s, DEMO_AGENT), NetEvent::ClassifiedInfo(Box::new(saved))]
        })),
        NetCommand::ClassifiedDelete(id) => {
            with(|s| s.retain(|c| c.id != *id));
            Some(Vec::new())
        }
        NetCommand::ClassifiedStatsRequest(id) => Some(vec![NetEvent::ClassifiedStats {
            id: *id,
            stats: ClassifiedStats {
                teleport: 12,
                map: 4,
                profile: 7,
            },
        }]),
        NetCommand::ClassifiedClick { .. } => Some(Vec::new()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_classified_is_paid_once_then_listed() {
        let new = ClassifiedInfo {
            id: Uuid::from_u128(0xC1A5_00F0),
            name: "Atelier ouvert".into(),
            price: 75,
            ..Default::default()
        };
        let cmd = NetCommand::ClassifiedUpdate(Box::new(new.clone()));
        assert_eq!(price_of_new(&cmd), Some(75));
        let events = reply(&cmd).expect("an answer");
        assert!(matches!(&events[0], NetEvent::AvatarClassifieds { target, list }
            if *target == DEMO_AGENT && list.iter().any(|(id, _)| *id == new.id)));
        // saved again: no new payment, the price of the publication stays
        let edit = NetCommand::ClassifiedUpdate(Box::new(ClassifiedInfo { price: 0, ..new.clone() }));
        assert_eq!(price_of_new(&edit), None);
        let events = reply(&edit).expect("an answer");
        assert!(matches!(&events[1], NetEvent::ClassifiedInfo(c) if c.price == 75));
        reply(&NetCommand::ClassifiedDelete(new.id));
        assert_eq!(price_of_new(&cmd), Some(75));
    }
}
