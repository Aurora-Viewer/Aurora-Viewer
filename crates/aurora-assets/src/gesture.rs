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

/// LLDataPackerAsciiBuffer::getValueStr without field names: a value is the
/// text up to the next newline or NUL, since its sscanf stops at either and
/// then steps over one byte (a NUL ends a value like a newline does). A value
/// past the end is refused: the Legacy Viewer reads beyond its buffer there.
struct Fields<'a>(Option<&'a [u8]>);

impl<'a> Fields<'a> {
    fn field(&mut self) -> Result<&'a [u8], AssetError> {
        let rest = self.0.take().ok_or(AssetError::Truncated("gesture field"))?;
        Ok(match rest.iter().position(|b| matches!(b, b'\n' | b'\0')) {
            Some(end) => {
                self.0 = Some(&rest[end + 1..]);
                &rest[..end]
            }
            None => rest,
        })
    }

    /// Not UTF-8 is read lossily. A carriage return, which an earlier
    /// gesture editor left at the end of animation names
    /// (LLGestureStepAnimation::deserialize) and Windows line ends leave
    /// anywhere, is dropped: a saved gesture cannot hold one.
    fn text(&mut self) -> Result<String, AssetError> {
        Ok(String::from_utf8_lossy(self.field()?).replace('\r', ""))
    }

    /// sscanf "%d" / "%u": blanks, a sign and digits, the rest ignored; no
    /// digit leaves the value unread. Out of range saturates, then the
    /// caller's `as` keeps the low bits like the C conversion.
    fn number(&mut self) -> Result<Option<i64>, AssetError> {
        let s = self.field()?.trim_ascii_start();
        let (negative, s) = match s.split_first() {
            Some((b'-', rest)) => (true, rest),
            Some((b'+', rest)) => (false, rest),
            _ => (false, s),
        };
        let digits = s.iter().take_while(|b| b.is_ascii_digit()).count();
        let value = s[..digits]
            .iter()
            .fold(0i64, |n, d| n.saturating_mul(10).saturating_add(i64::from(d - b'0')));
        Ok((digits > 0).then_some(if negative { -value } else { value }))
    }

    /// A U32 the Legacy Viewer initialises to 0 and leaves so when unread.
    fn unsigned(&mut self) -> Result<u32, AssetError> {
        Ok(self.number()?.map_or(0, |v| v as u32))
    }

    /// sscanf "%f": the decimal number at the start, the rest ignored
    /// (infinities and NaN, which no wait can use, are left unread).
    fn decimal(&mut self) -> Result<Option<f32>, AssetError> {
        let s = self.field()?.trim_ascii_start();
        let digits = |from: usize| s[from..].iter().take_while(|b| b.is_ascii_digit()).count();
        let mut end = usize::from(matches!(s.first(), Some(b'+' | b'-')));
        let whole = digits(end);
        end += whole;
        let mut fraction = 0;
        if s.get(end) == Some(&b'.') {
            fraction = digits(end + 1);
            end += 1 + fraction;
        }
        if whole + fraction == 0 {
            return Ok(None);
        }
        if matches!(s.get(end), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(s.get(end + 1), Some(b'+' | b'-')));
            let exponent = digits(end + 1 + sign);
            if exponent > 0 {
                end += 1 + sign + exponent;
            }
        }
        Ok(std::str::from_utf8(&s[..end]).ok().and_then(|n| n.parse().ok()))
    }

    /// sscanf "%63s" then LLUUID::set: the first word, nil when it is not a UUID.
    fn uuid(&mut self) -> Result<Uuid, AssetError> {
        let word = self.field()?.trim_ascii_start();
        let word = word.split(u8::is_ascii_whitespace).next().unwrap_or_default();
        Ok(Uuid::try_parse_ascii(&word[..word.len().min(63)]).unwrap_or_default())
    }
}

impl Gesture {
    /// LLMultiGesture::deserialize, as tolerant: only a version other than 2,
    /// a negative step count or an unknown step type fail (an unreadable one
    /// too: these are C locals the Legacy Viewer leaves uninitialised), and a
    /// value missing at the end. Another bad number is 0 (a key, none), a bad
    /// UUID nil, and what follows the last step is ignored. Aurora keeps its
    /// size limits, and a value whole where getValueStr cuts it at 511 bytes.
    /// Such a gesture is only written back when the resident saves it.
    pub fn decode(data: &[u8]) -> Result<Self, AssetError> {
        if data.len() > MAX_BYTES {
            return Err(AssetError::TooLarge("gesture"));
        }
        let mut fields = Fields(Some(data));
        if fields.number()?.map(|v| v as i32) != Some(2) {
            return Err(AssetError::Unsupported("gesture version".into()));
        }
        let key = fields.number()?.map_or(KEY_NONE, |v| v as u8);
        let mask = fields.unsigned()?;
        let trigger = fields.text()?;
        let replacement = fields.text()?;
        let count = fields.number()?.map(|v| v as i32).ok_or_else(invalid)?;
        let count = usize::try_from(count).map_err(|_| invalid())?;
        if count > MAX_STEPS {
            return Err(AssetError::TooLarge("gesture steps"));
        }
        let mut steps = Vec::with_capacity(count);
        for _ in 0..count {
            let step = match fields.number()?.map(|v| v as i32) {
                Some(kind @ (0 | 1)) => {
                    let name = fields.text()?;
                    let asset = fields.uuid()?;
                    let flags = fields.unsigned()?;
                    if kind == 0 {
                        Step::Animation { name, asset, flags }
                    } else {
                        Step::Sound { name, asset, flags }
                    }
                }
                Some(2) => Step::Chat {
                    text: fields.text()?,
                    flags: fields.unsigned()?,
                },
                // A negative wait passes at once in LLGestureMgr::stepGesture;
                // NaN or an infinity (which would never end) waits no time here.
                Some(3) => Step::Wait {
                    seconds: fields.decimal()?.filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(0.0),
                    flags: fields.unsigned()?,
                },
                _ => return Err(AssetError::Unsupported("gesture step".into())),
            };
            steps.push(step);
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
    /// LLMultiGesture::deserialize fails only on these (and Aurora on its
    /// size limits and a value missing at the end).
    #[test]
    fn rejects_invalid_external_data_without_panicking() {
        for text in [
            "",
            "2\n",
            "1\n255\n0\na\nb\n0\n",
            "x\n255\n0\na\nb\n0\n",
            "2\n255\n0\na\nb\n-1\n",
            "2\n255\n0\na\nb\nmany\n",
            "2\n255\n0\na\nb\n999999\n",
            "2\n255\n0\na\nb\n1\n9\n",
            "2\n255\n0\na\nb\n1\nchat\n",
            "2\n255\n0\na\nb\n1\n0\nname\n",
            "2\n255\n0\na\nb\n2\n2\nhi\n0\n",
        ] {
            assert!(Gesture::decode(text.as_bytes()).is_err(), "{text}");
        }
        let long = format!("2\n255\n0\n{}\nb\n0\n", "a".repeat(5000));
        assert!(Gesture::decode(long.as_bytes()).is_err());
        let g = Gesture {
            trigger: "a\nb".into(),
            ..Default::default()
        };
        assert!(g.encode().is_err());
    }

    /// Gestures the Legacy Viewer plays (#2500): LLDataPackerAsciiBuffer
    /// leaves its unpack results unchecked, so padding, a line after the
    /// last step, a Latin-1 name and bad numbers or UUIDs do not fail.
    #[test]
    fn decodes_like_the_legacy_viewer_with_trailing_data_and_padding() {
        let mut data = b"2\n255\n0\n/wave\n\n1\n0\nWave\r\n0a9fb970-8b44-9114-d3a9-bf69cfe804d6\n0\n\0\0".to_vec();
        let g = Gesture::decode(&data).expect("padded");
        assert_eq!(g.trigger, "/wave");
        assert_eq!(
            g.steps,
            [Step::Animation {
                name: "Wave".into(),
                asset: Uuid::parse_str("0a9fb970-8b44-9114-d3a9-bf69cfe804d6").expect("uuid"),
                flags: 0,
            }]
        );
        data.extend_from_slice(b"extra\n");
        assert_eq!(Gesture::decode(&data).expect("trailing line"), g);

        let latin1 = b"2\n255\n0\n/caf\xe9\n\n1\n1\nCaf\xe9\nnot a uuid\n7 flags\n";
        let g = Gesture::decode(latin1).expect("Latin-1");
        assert_eq!(g.trigger, "/caf\u{fffd}");
        assert_eq!(
            g.steps,
            [Step::Sound {
                name: "Caf\u{fffd}".into(),
                asset: Uuid::nil(),
                flags: 7,
            }]
        );

        // sscanf: a key wraps to its U8, unread numbers stay 0, a NUL ends a
        // value, and a wait that is not a time does not wait.
        let loose = b"2\n256\nmask\n/a\0/b\n3\n3\nNaN\n1\n3\n-1\nx\n3\n2.5s\n1\n";
        let g = Gesture::decode(loose).expect("loose numbers");
        assert_eq!((g.key, g.mask, g.trigger.as_str(), g.replacement.as_str()), (0, 0, "/a", "/b"));
        let wait = |seconds, flags| Step::Wait { seconds, flags };
        assert_eq!(g.steps, [wait(0.0, 1), wait(0.0, 0), wait(2.5, 1)]);
        assert_eq!(Gesture::decode(&g.encode().expect("saved")).expect("read back"), g);
    }
}
