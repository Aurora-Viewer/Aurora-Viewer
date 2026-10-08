//! Tiny XML-RPC codec mapping values to/from LLSD (used by login).

use aurora_llsd::xml::{self, Element};
use aurora_llsd::{Llsd, Map};
use base64::Engine as _;

fn write_value(v: &Llsd, out: &mut String) {
    out.push_str("<value>");
    match v {
        Llsd::Undef => out.push_str("<string></string>"),
        Llsd::Boolean(b) => {
            out.push_str("<boolean>");
            out.push_str(if *b { "1" } else { "0" });
            out.push_str("</boolean>");
        }
        Llsd::Integer(i) => {
            out.push_str("<int>");
            out.push_str(&i.to_string());
            out.push_str("</int>");
        }
        Llsd::Real(r) => {
            out.push_str("<double>");
            out.push_str(&r.to_string());
            out.push_str("</double>");
        }
        Llsd::String(s) | Llsd::Uri(s) => {
            out.push_str("<string>");
            xml::escape(s, out);
            out.push_str("</string>");
        }
        Llsd::Uuid(u) => {
            out.push_str("<string>");
            out.push_str(&u.to_string());
            out.push_str("</string>");
        }
        Llsd::Date(d) => {
            out.push_str("<double>");
            out.push_str(&d.to_string());
            out.push_str("</double>");
        }
        Llsd::Binary(b) => {
            out.push_str("<base64>");
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(b));
            out.push_str("</base64>");
        }
        Llsd::Array(a) => {
            out.push_str("<array><data>");
            for e in a {
                write_value(e, out);
            }
            out.push_str("</data></array>");
        }
        Llsd::Map(m) => {
            out.push_str("<struct>");
            for (k, e) in m {
                out.push_str("<member><name>");
                xml::escape(k, out);
                out.push_str("</name>");
                write_value(e, out);
                out.push_str("</member>");
            }
            out.push_str("</struct>");
        }
    }
    out.push_str("</value>");
}

/// Build a `methodCall` document with a single struct parameter.
pub fn build_call(method: &str, param: &Llsd) -> String {
    let mut s = String::from("<?xml version=\"1.0\"?><methodCall><methodName>");
    xml::escape(method, &mut s);
    s.push_str("</methodName><params><param>");
    write_value(param, &mut s);
    s.push_str("</param></params></methodCall>");
    s
}

fn parse_value(el: &Element) -> Llsd {
    // <value> may contain a typed child or bare text (string).
    let Some(child) = el.elements().next() else {
        return Llsd::String(el.text());
    };
    match child.name.as_str() {
        "string" => Llsd::String(child.text()),
        "int" | "i4" | "i8" => Llsd::Integer(child.text().trim().parse().unwrap_or(0)),
        "boolean" => Llsd::Boolean(child.text().trim() == "1"),
        "double" => Llsd::Real(child.text().trim().parse().unwrap_or(0.0)),
        "base64" => Llsd::Binary(
            base64::engine::general_purpose::STANDARD
                .decode(child.text().split_whitespace().collect::<String>())
                .unwrap_or_default(),
        ),
        "dateTime.iso8601" => Llsd::String(child.text()),
        "array" => {
            let mut a = Vec::new();
            if let Some(data) = child.child("data") {
                for v in data.elements().filter(|e| e.name == "value") {
                    a.push(parse_value(v));
                }
            }
            Llsd::Array(a)
        }
        "struct" => {
            let mut m = Map::new();
            for member in child.elements().filter(|e| e.name == "member") {
                let name = member.child("name").map(|n| n.text()).unwrap_or_default();
                let v = member.child("value").map(parse_value).unwrap_or_default();
                m.insert(name, v);
            }
            Llsd::Map(m)
        }
        "nil" => Llsd::Undef,
        _ => Llsd::String(child.text()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum XmlRpcError {
    #[error("xml parse error: {0}")]
    Xml(#[from] xml::XmlError),
    #[error("fault {code}: {message}")]
    Fault { code: i32, message: String },
    #[error("malformed response")]
    Malformed,
}

/// Parse a `methodResponse` and return its first parameter.
pub fn parse_response(body: &[u8]) -> Result<Llsd, XmlRpcError> {
    let root = xml::parse(body)?;
    if root.name != "methodResponse" {
        return Err(XmlRpcError::Malformed);
    }
    if let Some(fault) = root.child("fault") {
        let v = fault.child("value").map(parse_value).unwrap_or_default();
        return Err(XmlRpcError::Fault {
            code: v["faultCode"].as_i32(),
            message: v["faultString"].to_string_value(),
        });
    }
    let v = root
        .child("params")
        .and_then(|p| p.child("param"))
        .and_then(|p| p.child("value"))
        .ok_or(XmlRpcError::Malformed)?;
    Ok(parse_value(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn roundtrip() {
        let p = llsd_map! {"first" => "Test", "n" => 3, "opts" => Llsd::Array(vec!["a".into()]), "b" => true};
        let doc = build_call("login_to_simulator", &p);
        let resp = doc
            .replace("methodCall", "methodResponse")
            .replace("<methodName>login_to_simulator</methodName>", "");
        let v = parse_response(resp.as_bytes()).unwrap();
        assert_eq!(v["first"].as_str(), "Test");
        assert_eq!(v["n"].as_i32(), 3);
        assert_eq!(v["opts"][0].as_str(), "a");
        assert!(v["b"].as_bool());
    }

    #[test]
    fn bare_string_values() {
        let r = b"<?xml version='1.0'?><methodResponse><params><param><value><struct><member><name>login</name><value>true</value></member></struct></value></param></params></methodResponse>";
        let v = parse_response(r).unwrap();
        assert_eq!(v["login"].as_str(), "true");
    }

    #[test]
    fn fault() {
        let r = b"<methodResponse><fault><value><struct><member><name>faultCode</name><value><int>4</int></value></member><member><name>faultString</name><value><string>Too many</string></value></member></struct></value></fault></methodResponse>";
        assert!(matches!(parse_response(r), Err(XmlRpcError::Fault { code: 4, .. })));
    }
}
