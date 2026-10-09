//! LLSD <-> XML (`application/llsd+xml`).

use crate::xml::{self, Element};
use crate::{Llsd, LlsdError, Map};
use base64::Engine;
use uuid::Uuid;

pub(crate) fn format_date(secs: f64) -> String {
    let whole = secs.floor();
    let nanos = ((secs - whole) * 1e9).round().clamp(0.0, 999_999_999.0) as u32;
    match chrono::DateTime::from_timestamp(whole as i64, nanos) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        None => "1970-01-01T00:00:00Z".to_owned(),
    }
}

pub(crate) fn parse_date(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 * 1e-9;
    }
    // People API dates: "2010-04-16T21:34:02+00:00Z" (offset and Z)
    if let Some(t) = s
        .strip_suffix('Z')
        .filter(|t| t.len() > 6 && matches!(t.as_bytes()[t.len() - 6], b'+' | b'-'))
        && let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t)
    {
        return dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 * 1e-9;
    }
    // LL sometimes omits the timezone.
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        let u = dt.and_utc();
        return u.timestamp() as f64 + u.timestamp_subsec_nanos() as f64 * 1e-9;
    }
    0.0
}

fn decode_binary(el: &Element) -> Vec<u8> {
    let txt: String = el.text().chars().filter(|c| !c.is_whitespace()).collect();
    match el.attr("encoding").unwrap_or("base64") {
        "base16" => (0..txt.len() / 2)
            .filter_map(|i| u8::from_str_radix(txt.get(i * 2..i * 2 + 2)?, 16).ok())
            .collect(),
        _ => base64::engine::general_purpose::STANDARD
            .decode(txt.as_bytes())
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(txt.as_bytes()))
            .unwrap_or_default(),
    }
}

/// Convert a parsed XML value element (e.g. `<map>`) into LLSD.
pub fn from_xml_element(el: &Element) -> Result<Llsd, LlsdError> {
    let text = || el.text();
    Ok(match el.name.as_str() {
        "llsd" => match el.elements().next() {
            Some(c) => from_xml_element(c)?,
            None => Llsd::Undef,
        },
        "undef" => Llsd::Undef,
        "boolean" => {
            let t = text();
            let t = t.trim();
            Llsd::Boolean(t == "1" || t.eq_ignore_ascii_case("true"))
        }
        "integer" => {
            let t = text();
            let t = t.trim();
            Llsd::Integer(t.parse::<i32>().or_else(|_| t.parse::<f64>().map(|f| f as i32)).unwrap_or(0))
        }
        "real" => {
            let t = text();
            let t = t.trim();
            Llsd::Real(match t {
                "nan" | "NaN" => f64::NAN,
                "inf" | "+inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                _ => t.parse().unwrap_or(0.0),
            })
        }
        "string" => Llsd::String(text()),
        "uuid" => Llsd::Uuid(Uuid::parse_str(text().trim()).unwrap_or(Uuid::nil())),
        "date" => Llsd::Date(parse_date(&text())),
        "uri" => Llsd::Uri(text()),
        "binary" => Llsd::Binary(decode_binary(el)),
        "array" => {
            let mut a = Vec::new();
            for c in el.elements() {
                a.push(from_xml_element(c)?);
            }
            Llsd::Array(a)
        }
        "map" => {
            let mut m = Map::new();
            let mut key: Option<String> = None;
            for c in el.elements() {
                if c.name == "key" {
                    key = Some(c.text());
                } else if let Some(k) = key.take() {
                    m.insert(k, from_xml_element(c)?);
                } else {
                    return Err(LlsdError::Invalid("map value without key"));
                }
            }
            Llsd::Map(m)
        }
        _ => return Err(LlsdError::Invalid("unknown LLSD element")),
    })
}

/// Parse an `<llsd>` XML document.
pub fn from_xml(src: &[u8]) -> Result<Llsd, LlsdError> {
    let root = xml::parse(src)?;
    from_xml_element(&root)
}

fn write_value(v: &Llsd, out: &mut String) {
    match v {
        Llsd::Undef => out.push_str("<undef />"),
        Llsd::Boolean(b) => {
            out.push_str(if *b { "<boolean>1</boolean>" } else { "<boolean>0</boolean>" });
        }
        Llsd::Integer(i) => {
            out.push_str("<integer>");
            out.push_str(&i.to_string());
            out.push_str("</integer>");
        }
        Llsd::Real(r) => {
            out.push_str("<real>");
            if r.is_nan() {
                out.push_str("nan");
            } else if r.is_infinite() {
                out.push_str(if *r > 0.0 { "inf" } else { "-inf" });
            } else {
                out.push_str(&r.to_string());
            }
            out.push_str("</real>");
        }
        Llsd::String(s) => {
            out.push_str("<string>");
            xml::escape(s, out);
            out.push_str("</string>");
        }
        Llsd::Uuid(u) => {
            out.push_str("<uuid>");
            out.push_str(&u.to_string());
            out.push_str("</uuid>");
        }
        Llsd::Date(d) => {
            out.push_str("<date>");
            out.push_str(&format_date(*d));
            out.push_str("</date>");
        }
        Llsd::Uri(s) => {
            out.push_str("<uri>");
            xml::escape(s, out);
            out.push_str("</uri>");
        }
        Llsd::Binary(b) => {
            out.push_str("<binary encoding=\"base64\">");
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(b));
            out.push_str("</binary>");
        }
        Llsd::Array(a) => {
            if a.is_empty() {
                out.push_str("<array />");
                return;
            }
            out.push_str("<array>");
            for e in a {
                write_value(e, out);
            }
            out.push_str("</array>");
        }
        Llsd::Map(m) => {
            if m.is_empty() {
                out.push_str("<map />");
                return;
            }
            out.push_str("<map>");
            for (k, e) in m {
                out.push_str("<key>");
                xml::escape(k, out);
                out.push_str("</key>");
                write_value(e, out);
            }
            out.push_str("</map>");
        }
    }
}

pub fn to_xml_string(v: &Llsd) -> String {
    let mut s = String::from("<?xml version=\"1.0\" ?><llsd>");
    write_value(v, &mut s);
    s.push_str("</llsd>");
    s
}

pub fn to_xml(v: &Llsd) -> Vec<u8> {
    to_xml_string(v).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llsd_map;

    #[test]
    fn roundtrip() {
        let mut v = llsd_map! {
            "a" => 1, "b" => "x<y", "c" => 2.5, "d" => true,
            "e" => Uuid::from_u128(0x1234), "f" => vec![1u8, 2, 3],
        };
        v.insert("g", Llsd::Array(vec![Llsd::Undef, Llsd::Integer(5)]));
        let xml = to_xml(&v);
        let back = from_xml(&xml).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn parses_ll_style() {
        let doc = b"<?xml version=\"1.0\" ?>\n<llsd>\n<map>\n  <key>events</key>\n  <array>\n    <map><key>message</key><string>Foo</string></map>\n  </array>\n  <key>id</key><integer>3</integer>\n</map>\n</llsd>";
        let v = from_xml(doc).unwrap();
        assert_eq!(v["id"].as_i32(), 3);
        assert_eq!(v["events"][0]["message"].as_str(), "Foo");
    }
}
