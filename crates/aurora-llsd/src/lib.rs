//! LLSD (Linden Lab Structured Data) value type with XML and binary codecs.
//!
//! Port of the semantics of `llcommon/llsd.cpp` and `llsdserialize*.cpp`
//! from the Second Life viewer (originally LGPL 2.1).

pub mod binary;
pub mod xml;
mod xml_llsd;

use std::collections::BTreeMap;
use uuid::Uuid;

pub use binary::{from_binary, to_binary};
pub use xml_llsd::{from_xml, from_xml_element, to_xml, to_xml_string};

#[derive(Debug, thiserror::Error)]
pub enum LlsdError {
    #[error("xml: {0}")]
    Xml(#[from] xml::XmlError),
    #[error("invalid llsd: {0}")]
    Invalid(&'static str),
    #[error("unexpected end of data")]
    Eof,
}

pub type Map = BTreeMap<String, Llsd>;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Llsd {
    #[default]
    Undef,
    Boolean(bool),
    Integer(i32),
    Real(f64),
    String(String),
    Uuid(Uuid),
    /// Seconds since the Unix epoch.
    Date(f64),
    Uri(String),
    Binary(Vec<u8>),
    Array(Vec<Llsd>),
    Map(Map),
}

static UNDEF: Llsd = Llsd::Undef;

impl Llsd {
    pub fn new_map() -> Self {
        Llsd::Map(Map::new())
    }

    pub fn new_array() -> Self {
        Llsd::Array(Vec::new())
    }

    pub fn is_undef(&self) -> bool {
        matches!(self, Llsd::Undef)
    }

    pub fn is_map(&self) -> bool {
        matches!(self, Llsd::Map(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Llsd::Array(_))
    }

    /// Map lookup; returns `Undef` when missing or not a map.
    pub fn get(&self, key: &str) -> &Llsd {
        match self {
            Llsd::Map(m) => m.get(key).unwrap_or(&UNDEF),
            _ => &UNDEF,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        matches!(self, Llsd::Map(m) if m.contains_key(key))
    }

    /// Array lookup; returns `Undef` when missing or not an array.
    pub fn at(&self, idx: usize) -> &Llsd {
        match self {
            Llsd::Array(a) => a.get(idx).unwrap_or(&UNDEF),
            _ => &UNDEF,
        }
    }

    pub fn as_array(&self) -> &[Llsd] {
        match self {
            Llsd::Array(a) => a,
            _ => &[],
        }
    }

    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Llsd::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Llsd::Array(a) => a.len(),
            Llsd::Map(m) => m.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Insert into a map (converts `Undef` into a map first).
    pub fn insert(&mut self, key: impl Into<String>, v: impl Into<Llsd>) {
        if self.is_undef() {
            *self = Llsd::new_map();
        }
        if let Llsd::Map(m) = self {
            m.insert(key.into(), v.into());
        }
    }

    /// Append to an array (converts `Undef` into an array first).
    pub fn push(&mut self, v: impl Into<Llsd>) {
        if self.is_undef() {
            *self = Llsd::new_array();
        }
        if let Llsd::Array(a) = self {
            a.push(v.into());
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Llsd::Boolean(b) => *b,
            Llsd::Integer(i) => *i != 0,
            Llsd::Real(r) => *r != 0.0,
            Llsd::String(s) => !(s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false")),
            _ => false,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Llsd::Boolean(b) => *b as i32,
            Llsd::Integer(i) => *i,
            Llsd::Real(r) => r.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32,
            Llsd::String(s) => s
                .trim()
                .parse::<i64>()
                .map(|v| v as i32)
                .or_else(|_| s.trim().parse::<f64>().map(|v| v as i32))
                .unwrap_or(0),
            Llsd::Binary(b) if b.len() >= 4 => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            _ => 0,
        }
    }

    /// Unsigned interpretation (used for U32 values packed into integers).
    pub fn as_u32(&self) -> u32 {
        match self {
            Llsd::Real(r) => r.max(0.0).min(u32::MAX as f64) as u32,
            Llsd::String(s) => s.trim().parse::<u64>().map(|v| v as u32).unwrap_or(0),
            other => other.as_i32() as u32,
        }
    }

    /// U64 values are transported as 8-byte binaries by convention.
    pub fn as_u64(&self) -> u64 {
        match self {
            Llsd::Binary(b) if b.len() >= 8 => u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            Llsd::String(s) => s.trim().parse().unwrap_or(0),
            Llsd::Real(r) => *r as u64,
            other => other.as_i32() as u32 as u64,
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Llsd::Boolean(b) => *b as i32 as f64,
            Llsd::Integer(i) => *i as f64,
            Llsd::Real(r) => *r,
            Llsd::Date(d) => *d,
            Llsd::String(s) => s.trim().parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn as_f32(&self) -> f32 {
        self.as_f64() as f32
    }

    pub fn as_str(&self) -> &str {
        match self {
            Llsd::String(s) | Llsd::Uri(s) => s,
            _ => "",
        }
    }

    /// String conversion following LLSD rules.
    pub fn to_string_value(&self) -> String {
        match self {
            Llsd::Undef => String::new(),
            Llsd::Boolean(b) => if *b { "true" } else { "" }.to_owned(),
            Llsd::Integer(i) => i.to_string(),
            Llsd::Real(r) => r.to_string(),
            Llsd::String(s) | Llsd::Uri(s) => s.clone(),
            Llsd::Uuid(u) => u.to_string(),
            Llsd::Date(d) => xml_llsd::format_date(*d),
            Llsd::Binary(b) => String::from_utf8_lossy(b).into_owned(),
            Llsd::Array(_) | Llsd::Map(_) => String::new(),
        }
    }

    pub fn as_uuid(&self) -> Uuid {
        match self {
            Llsd::Uuid(u) => *u,
            Llsd::String(s) => Uuid::parse_str(s.trim()).unwrap_or(Uuid::nil()),
            Llsd::Binary(b) if b.len() == 16 => Uuid::from_slice(b).unwrap_or(Uuid::nil()),
            _ => Uuid::nil(),
        }
    }

    pub fn as_binary(&self) -> &[u8] {
        match self {
            Llsd::Binary(b) => b,
            Llsd::String(s) => s.as_bytes(),
            _ => &[],
        }
    }

    /// Interpret an LLSD array of 3 reals as a vector.
    pub fn as_vec3(&self) -> [f32; 3] {
        [self.at(0).as_f32(), self.at(1).as_f32(), self.at(2).as_f32()]
    }

    pub fn as_vec4(&self) -> [f32; 4] {
        [self.at(0).as_f32(), self.at(1).as_f32(), self.at(2).as_f32(), self.at(3).as_f32()]
    }
}

impl std::ops::Index<&str> for Llsd {
    type Output = Llsd;
    fn index(&self, key: &str) -> &Llsd {
        self.get(key)
    }
}

impl std::ops::Index<usize> for Llsd {
    type Output = Llsd;
    fn index(&self, idx: usize) -> &Llsd {
        self.at(idx)
    }
}

impl From<bool> for Llsd {
    fn from(v: bool) -> Self {
        Llsd::Boolean(v)
    }
}
impl From<i32> for Llsd {
    fn from(v: i32) -> Self {
        Llsd::Integer(v)
    }
}
impl From<u32> for Llsd {
    fn from(v: u32) -> Self {
        Llsd::Integer(v as i32)
    }
}
impl From<f64> for Llsd {
    fn from(v: f64) -> Self {
        Llsd::Real(v)
    }
}
impl From<f32> for Llsd {
    fn from(v: f32) -> Self {
        Llsd::Real(v as f64)
    }
}
impl From<&str> for Llsd {
    fn from(v: &str) -> Self {
        Llsd::String(v.to_owned())
    }
}
impl From<String> for Llsd {
    fn from(v: String) -> Self {
        Llsd::String(v)
    }
}
impl From<Uuid> for Llsd {
    fn from(v: Uuid) -> Self {
        Llsd::Uuid(v)
    }
}
impl From<Vec<Llsd>> for Llsd {
    fn from(v: Vec<Llsd>) -> Self {
        Llsd::Array(v)
    }
}
impl From<Map> for Llsd {
    fn from(v: Map) -> Self {
        Llsd::Map(v)
    }
}
impl From<Vec<u8>> for Llsd {
    fn from(v: Vec<u8>) -> Self {
        Llsd::Binary(v)
    }
}

/// Build an LLSD map literal: `llsd_map! { "a" => 1, "b" => "x" }`.
#[macro_export]
macro_rules! llsd_map {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = $crate::Map::new();
        $( m.insert(($k).to_string(), $crate::Llsd::from($v)); )*
        $crate::Llsd::Map(m)
    }};
}
