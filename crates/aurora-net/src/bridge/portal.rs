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
    pub fn valid(&self) -> bool {
        !self.id.is_nil()
            && self.region != 0
            && self.center.iter().all(|v| v.is_finite())
            && (0.0..=8192.0).contains(&self.center[0])
            && (0.0..=8192.0).contains(&self.center[1])
            && (0.0..=4096.0).contains(&self.center[2])
            && self.yaw.is_finite()
            && self.yaw.abs() <= std::f32::consts::TAU
            && !self.destination.trim().is_empty()
            && self.destination.len() <= 128
            && !self.destination.chars().any(|c| c.is_control() || "/\\:|".contains(c))
            && self.position.iter().all(|v| v.is_finite())
            && (0.0..=8192.0).contains(&self.position[0])
            && (0.0..=8192.0).contains(&self.position[1])
            && (0.0..=4096.0).contains(&self.position[2])
            && !matches!(self.access, Access::Group(id) if id.is_nil())
            && self.expires > 0
    }

    pub fn live(&self, now: u64) -> bool {
        self.valid() && self.expires > now && self.expires.saturating_sub(now) <= LIFETIME + CLOCK_SKEW
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Open(Portal),
    Close { region: u64, id: Uuid },
    Discover { region: u64 },
}

impl Message {
    pub fn region(&self) -> u64 {
        match self {
            Self::Open(p) => p.region,
            Self::Close { region, .. } | Self::Discover { region } => *region,
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
        let message = Message::Open(p.clone());
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
}
