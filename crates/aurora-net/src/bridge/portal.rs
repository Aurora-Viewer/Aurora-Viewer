//! Bounded region-local portal invitations. The relay supplies the owner's
//! identity; a payload can never choose who is teleported.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const LIFETIME: u64 = 120;
pub const CLOCK_SKEW: u64 = 30;
pub const MAX_WIRE: usize = 640;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access {
    #[default]
    Everyone,
    Friends,
    Group(Uuid),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub region: String,
    pub position: [f32; 3],
}

impl Destination {
    fn valid(&self) -> bool {
        !self.region.trim().is_empty()
            && self.region.len() <= 128
            && !self.region.chars().any(|c| c.is_control() || "/\\:|".contains(c))
            && self.position.iter().all(|v| v.is_finite())
            && (0.0..=8192.0).contains(&self.position[0])
            && (0.0..=8192.0).contains(&self.position[1])
            && (0.0..=4096.0).contains(&self.position[2])
    }
}

/// The only broadcastable representation. Restricted portals omit all
/// destination data, including the landmark asset and the group's identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Announcement {
    pub id: Uuid,
    /// Return relays retain the original creator for access checks and labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<Uuid>,
    pub region: u64,
    pub center: [f32; 3],
    pub yaw: f32,
    pub expires: u64,
    pub destination: Option<Destination>,
}

impl Announcement {
    pub fn valid(&self) -> bool {
        !self.id.is_nil()
            && self.creator.is_none_or(|id| !id.is_nil())
            && self.region != 0
            && self.center.iter().all(|v| v.is_finite())
            && (0.0..=8192.0).contains(&self.center[0])
            && (0.0..=8192.0).contains(&self.center[1])
            && (0.0..=4096.0).contains(&self.center[2])
            && self.yaw.is_finite()
            && self.yaw.abs() <= std::f32::consts::TAU
            && self.destination.as_ref().is_none_or(Destination::valid)
            && self.expires > 0
    }

    pub fn live(&self, now: u64) -> bool {
        self.valid() && self.expires > now && self.expires.saturating_sub(now) <= LIFETIME + CLOCK_SKEW
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Details {
    pub destination: Destination,
    pub access: Access,
}

impl Details {
    fn valid(&self) -> bool {
        self.destination.valid() && !matches!(self.access, Access::Group(id) if id.is_nil())
    }
}

/// Full local creation data; deliberately not serializable as an announcement.
#[derive(Clone, Debug, PartialEq)]
pub struct Portal {
    pub id: Uuid,
    pub region: u64,
    pub center: [f32; 3],
    pub yaw: f32,
    pub destination: String,
    pub position: [f32; 3],
    pub access: Access,
    pub expires: u64,
}

impl Portal {
    pub fn details(&self) -> Details {
        Details {
            destination: Destination {
                region: self.destination.clone(),
                position: self.position,
            },
            access: self.access,
        }
    }

    pub fn announcement(&self) -> Announcement {
        Announcement {
            id: self.id,
            creator: None,
            region: self.region,
            center: self.center,
            yaw: self.yaw,
            expires: self.expires,
            destination: (self.access == Access::Everyone).then(|| self.details().destination),
        }
    }

    pub fn valid(&self) -> bool {
        self.announcement().valid() && self.details().valid()
    }

    pub fn live(&self, now: u64) -> bool {
        self.valid() && self.expires > now && self.expires.saturating_sub(now) <= LIFETIME + CLOCK_SKEW
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Open(Announcement),
    Close {
        region: u64,
        id: Uuid,
    },
    Discover {
        region: u64,
    },
    Request {
        region: u64,
        id: Uuid,
        nonce: Uuid,
    },
    Details {
        region: u64,
        id: Uuid,
        nonce: Uuid,
        details: Details,
    },
}

impl Message {
    pub fn region(&self) -> u64 {
        match self {
            Self::Open(p) => p.region,
            Self::Close { region, .. } | Self::Discover { region } | Self::Request { region, .. } | Self::Details { region, .. } => *region,
        }
    }
    pub fn encode(&self) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let wire = STANDARD.encode(serde_json::to_vec(self).ok()?);
        (wire.len() <= MAX_WIRE).then_some(wire)
    }
    pub fn decode(wire: &str) -> Option<Self> {
        if wire.len() > MAX_WIRE {
            return None;
        }
        let message: Self = serde_json::from_slice(&STANDARD.decode(wire).ok()?).ok()?;
        message.valid().then_some(message)
    }
    fn valid(&self) -> bool {
        self.region() != 0
            && match self {
                Self::Open(p) => p.valid(),
                Self::Close { id, .. } => !id.is_nil(),
                Self::Discover { .. } => true,
                Self::Request { id, nonce, .. } => !id.is_nil() && !nonce.is_nil(),
                Self::Details { id, nonce, details, .. } => {
                    !id.is_nil() && !nonce.is_nil() && details.valid() && details.access != Access::Everyone
                }
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> Portal {
        Portal {
            id: Uuid::from_u128(7),
            region: 256,
            center: [10.0, 12.0, 24.0],
            yaw: 0.0,
            destination: "Demo destination".into(),
            position: [128.0, 128.0, 25.0],
            access: Access::Everyone,
            expires: 1120,
        }
    }

    #[test]
    fn portal_wire_rejects_unbounded_malformed_and_cross_grid_destinations() {
        let p = sample();
        let message = Message::Open(p.announcement());
        assert_eq!(Message::decode(&message.encode().unwrap()), Some(message));
        assert!(p.live(1000));
        assert!(p.live(999));
        assert!(!p.live(969));
        assert!(!p.live(1120));
        assert!(Message::decode(&"A".repeat(MAX_WIRE + 1)).is_none());
        assert!(Message::decode("not a payload").is_none());
        for destination in ["", "grid:region", "https://example.invalid", "region\nname"] {
            assert!(
                !Portal {
                    destination: destination.into(),
                    ..p.clone()
                }
                .valid()
            );
        }
        assert!(
            !Portal {
                center: [f32::NAN, 0.0, 0.0],
                ..p.clone()
            }
            .valid()
        );
        assert!(
            !Portal {
                access: Access::Group(Uuid::nil()),
                ..p
            }
            .valid()
        );
    }

    #[test]
    fn restricted_announcements_omit_destinations_and_group_identity_and_details_require_a_target() {
        for access in [Access::Friends, Access::Group(Uuid::from_u128(321))] {
            let p = Portal { access, ..sample() };
            let wire = Message::Open(p.announcement()).encode().unwrap();
            let json = String::from_utf8(STANDARD.decode(&wire).unwrap()).unwrap();
            assert!(!json.contains(&p.destination));
            assert!(!json.contains("position"));
            assert!(!json.contains("access"));
            assert!(!json.contains("00000000-0000-0000-0000-000000000141"));
            let Message::Open(announcement) = Message::decode(&wire).unwrap() else {
                panic!("announcement")
            };
            assert!(announcement.destination.is_none());
            assert_eq!(announcement.id, p.id);
            let message = Message::Details {
                region: p.region,
                id: p.id,
                nonce: Uuid::from_u128(9),
                details: p.details(),
            };
            assert_eq!(Message::decode(&message.encode().unwrap()), Some(message.clone()));
            assert!(
                !super::super::query::Query::Portal {
                    target: Uuid::nil(),
                    message: message.clone()
                }
                .valid()
            );
            assert!(
                super::super::query::Query::Portal {
                    target: Uuid::from_u128(2),
                    message
                }
                .valid()
            );
            assert!(
                !super::super::query::Query::Portal {
                    target: Uuid::nil(),
                    message: Message::Request {
                        region: p.region,
                        id: p.id,
                        nonce: Uuid::from_u128(9)
                    }
                }
                .valid()
            );
        }
        let public = sample().announcement();
        assert_eq!(public.destination.unwrap().region, sample().destination);
        let legacy = STANDARD.encode(br#"{"Open":{"id":"00000000-0000-0000-0000-000000000007","region":256,"center":[10,12,24],"yaw":0,"destination":"Private place","position":[128,128,25],"access":"Friends","expires":1120}}"#);
        assert!(Message::decode(&legacy).is_none());
    }
}
