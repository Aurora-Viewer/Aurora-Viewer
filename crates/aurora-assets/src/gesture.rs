//! LLMultiGesture v2 / LLDataPackerAsciiBuffer (Firestorm llcharacter,
//! originally LGPL 2.1). The asset contains values separated by newlines,
//! without field labels; an optional final NUL is part of legacy uploads.

use crate::AssetError;
use uuid::Uuid;

pub const KEY_NONE: u8 = 255;
pub const ANIM_STOP: u32 = 1;
pub const WAIT_TIME: u32 = 1;
pub const WAIT_ANIMATIONS: u32 = 2;
pub const WAIT_RELEASE: u32 = 4;
pub const MAX_STEPS: usize = 1024;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Gesture {
    pub key: u8,
    pub mask: u32,
    pub trigger: String,
    pub replacement: String,
    pub steps: Vec<Step>,
}

impl Default for Gesture {
    fn default() -> Self {
        Self {
            key: KEY_NONE,
            mask: 0,
            trigger: String::new(),
            replacement: String::new(),
            steps: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Animation { name: String, asset: Uuid, flags: u32 },
    Sound { name: String, asset: Uuid, flags: u32 },
    Chat { text: String, flags: u32 },
    Wait { seconds: f32, flags: u32 },
}

fn invalid() -> AssetError {
    AssetError::invalid("malformed gesture")
}

impl Gesture {
    pub fn decode(data: &[u8]) -> Result<Self, AssetError> {
        if data.len() > MAX_BYTES {
            return Err(AssetError::TooLarge("gesture"));
        }
        let data = data.strip_suffix(&[0]).unwrap_or(data);
        let text = std::str::from_utf8(data).map_err(|_| invalid())?;
        if text.contains('\0') {
            return Err(invalid());
        }
        let mut lines = text.lines();
        let mut next = || lines.next().ok_or(AssetError::Truncated("gesture field"));
        if next()?.trim() != "2" {
            return Err(AssetError::Unsupported("gesture version".into()));
        }
        let key = next()?.trim().parse().map_err(|_| invalid())?;
        let mask = next()?.trim().parse().map_err(|_| invalid())?;
        let trigger = next()?.to_owned();
        let replacement = next()?.to_owned();
        let count: usize = next()?.trim().parse().map_err(|_| invalid())?;
        if count > MAX_STEPS {
            return Err(AssetError::TooLarge("gesture steps"));
        }
        let mut steps = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = next()?.trim().parse::<u32>().map_err(|_| invalid())?;
            let step = match kind {
                0 | 1 => {
                    let name = next()?.to_owned();
                    let asset = Uuid::parse_str(next()?.trim()).map_err(|_| invalid())?;
                    let flags = next()?.trim().parse().map_err(|_| invalid())?;
                    if kind == 0 {
                        Step::Animation { name, asset, flags }
                    } else {
                        Step::Sound { name, asset, flags }
                    }
                }
                2 => Step::Chat {
                    text: next()?.to_owned(),
                    flags: next()?.trim().parse().map_err(|_| invalid())?,
                },
                3 => {
                    let seconds: f32 = next()?.trim().parse().map_err(|_| invalid())?;
                    if !seconds.is_finite() || seconds < 0.0 {
                        return Err(invalid());
                    }
                    Step::Wait {
                        seconds,
                        flags: next()?.trim().parse().map_err(|_| invalid())?,
                    }
                }
                _ => return Err(AssetError::Unsupported("gesture step".into())),
            };
            steps.push(step);
        }
        if lines.any(|line| !line.trim().is_empty()) {
            return Err(invalid());
        }
        let gesture = Self {
            key,
            mask,
            trigger,
            replacement,
            steps,
        };
        gesture.validate()?;
        Ok(gesture)
    }

    pub fn validate(&self) -> Result<(), AssetError> {
        let valid = |s: &str| s.len() <= 4096 && !s.contains(['\n', '\r', '\0']);
        if self.steps.len() > MAX_STEPS || !valid(&self.trigger) || !valid(&self.replacement) {
            return Err(invalid());
        }
        for step in &self.steps {
            let ok = match step {
                Step::Animation { name, .. } | Step::Sound { name, .. } => valid(name),
                Step::Chat { text, .. } => valid(text),
                Step::Wait { seconds, .. } => seconds.is_finite() && *seconds >= 0.0,
            };
            if !ok {
                return Err(invalid());
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, AssetError> {
        use std::fmt::Write;
        self.validate()?;
        let mut out = format!(
            "2\n{}\n{}\n{}\n{}\n{}\n",
            self.key,
            self.mask,
            self.trigger,
            self.replacement,
            self.steps.len()
        );
        for step in &self.steps {
            match step {
                Step::Animation { name, asset, flags } => {
                    let _ = writeln!(out, "0\n{name}\n{asset}\n{flags}");
                }
                Step::Sound { name, asset, flags } => {
                    let _ = writeln!(out, "1\n{name}\n{asset}\n{flags}");
                }
                Step::Chat { text, flags } => {
                    let _ = writeln!(out, "2\n{text}\n{flags}");
                }
                Step::Wait { seconds, flags } => {
                    let _ = writeln!(out, "3\n{seconds:.6}\n{flags}");
                }
            }
        }
        out.push('\0');
        if out.len() > MAX_BYTES {
            return Err(AssetError::TooLarge("gesture"));
        }
        Ok(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firestorm_asset_and_all_steps_roundtrip() {
        let original = b"2\n255\n0\n/extinguish\n\n1\n0\nSmoke Throw Down\n0a9fb970-8b44-9114-d3a9-bf69cfe804d6\n0\n\0";
        let mut g = Gesture::decode(original).expect("Firestorm fixture");
        assert_eq!(g.trigger, "/extinguish");
        g.steps.extend([
            Step::Animation {
                name: "stop".into(),
                asset: Uuid::nil(),
                flags: ANIM_STOP,
            },
            Step::Sound {
                name: "son".into(),
                asset: Uuid::from_u128(2),
                flags: 7,
            },
            Step::Chat {
                text: "/28574 T4 é".into(),
                flags: 0,
            },
            Step::Wait {
                seconds: 2.5,
                flags: WAIT_TIME | WAIT_RELEASE,
            },
        ]);
        assert_eq!(Gesture::decode(&g.encode().expect("encode")).expect("decode"), g);
    }
    #[test]
    fn rejects_invalid_external_data_without_panicking() {
        for text in [
            "",
            "2\n",
            "1\n255\n0\na\nb\n0\n",
            "2\n256\n0\na\nb\n0\n",
            "2\n255\n0\na\nb\n999999\n",
            "2\n255\n0\na\nb\n1\n3\nNaN\n1\n",
            "2\n255\n0\na\nb\n1\n3\n-1\n1\n",
            "2\n255\n0\na\nb\n1\n9\n",
            "2\n255\n0\na\nb\n0\nextra\n",
        ] {
            assert!(Gesture::decode(text.as_bytes()).is_err(), "{text}");
        }
        let g = Gesture {
            trigger: "a\nb".into(),
            ..Default::default()
        };
        assert!(g.encode().is_err());
    }
}
