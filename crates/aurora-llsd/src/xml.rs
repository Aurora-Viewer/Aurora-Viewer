//! Minimal, allocation-light XML reader used for LLSD+XML and XML-RPC.
//!
//! Supports elements, attributes, text, CDATA, comments, processing
//! instructions, DOCTYPE skipping and the predefined/numeric entities.
//! It is intentionally strict about structure but tolerant about whitespace.

use std::fmt;

const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    pub pos: usize,
    pub msg: &'static str,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "XML error at byte {}: {}", self.pos, self.msg)
    }
}

impl std::error::Error for XmlError {}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Iterate child elements, skipping text nodes.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    /// Concatenated text content of direct text children.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.children {
            if let Node::Text(t) = n {
                s.push_str(t);
            }
        }
        s
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &'static str) -> XmlError {
        XmlError { pos: self.pos, msg }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn starts_with(&self, s: &[u8]) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_until(&mut self, pat: &[u8]) -> Result<(), XmlError> {
        while self.pos < self.src.len() {
            if self.starts_with(pat) {
                self.pos += pat.len();
                return Ok(());
            }
            self.pos += 1;
        }
        Err(self.err("unterminated construct"))
    }

    /// Skip prolog items: declarations, comments, PIs, doctype, whitespace.
    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_ws();
            if self.starts_with(b"<?") {
                self.skip_until(b"?>")?;
            } else if self.starts_with(b"<!--") {
                self.skip_until(b"-->")?;
            } else if self.starts_with(b"<!DOCTYPE") || self.starts_with(b"<!doctype") {
                // Handle optional internal subset.
                let mut depth = 0i32;
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    match c {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => break,
                        _ => {}
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() || c == b'>' || c == b'/' || c == b'=' {
                break;
            }
            self.pos += 1;
        }
        if start == self.pos {
            return Err(self.err("expected name"));
        }
        Ok(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    fn element(&mut self, depth: usize) -> Result<Element, XmlError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        if self.peek() != Some(b'<') {
            return Err(self.err("expected '<'"));
        }
        self.pos += 1;
        let name = self.name()?;
        let mut el = Element {
            name,
            ..Default::default()
        };
        // attributes
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.pos += 1;
                    if self.peek() != Some(b'>') {
                        return Err(self.err("expected '>' after '/'"));
                    }
                    self.pos += 1;
                    return Ok(el);
                }
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => {
                    let k = self.name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return Err(self.err("expected '='"));
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let q = self.peek().ok_or_else(|| self.err("eof in attribute"))?;
                    if q != b'"' && q != b'\'' {
                        return Err(self.err("expected quote"));
                    }
                    self.pos += 1;
                    let start = self.pos;
                    while self.peek().is_some_and(|c| c != q) {
                        self.pos += 1;
                    }
                    if self.peek().is_none() {
                        return Err(self.err("eof in attribute value"));
                    }
                    let raw = &self.src[start..self.pos];
                    self.pos += 1;
                    el.attrs.push((k, unescape(raw)));
                }
                None => return Err(self.err("eof in tag")),
            }
        }
        // content
        let mut text = Vec::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(self.err("eof in element content"));
            }
            if self.starts_with(b"</") {
                flush_text(&mut text, &mut el);
                self.pos += 2;
                let end = self.name()?;
                if end != el.name {
                    return Err(self.err("mismatched end tag"));
                }
                self.skip_ws();
                if self.peek() != Some(b'>') {
                    return Err(self.err("expected '>'"));
                }
                self.pos += 1;
                return Ok(el);
            } else if self.starts_with(b"<![CDATA[") {
                self.pos += 9;
                let start = self.pos;
                self.skip_until(b"]]>")?;
                text.extend_from_slice(&self.src[start..self.pos - 3]);
            } else if self.starts_with(b"<!--") {
                self.skip_until(b"-->")?;
            } else if self.starts_with(b"<?") {
                self.skip_until(b"?>")?;
            } else if self.peek() == Some(b'<') {
                flush_text(&mut text, &mut el);
                let child = self.element(depth + 1)?;
                el.children.push(Node::Element(child));
            } else {
                let start = self.pos;
                while self.peek().is_some_and(|c| c != b'<') {
                    self.pos += 1;
                }
                let decoded = unescape(&self.src[start..self.pos]);
                text.extend_from_slice(decoded.as_bytes());
            }
        }
    }
}

fn flush_text(text: &mut Vec<u8>, el: &mut Element) {
    if !text.is_empty() {
        let s = String::from_utf8_lossy(text).into_owned();
        el.children.push(Node::Text(s));
        text.clear();
    }
}

/// Decode XML entities in raw text.
pub fn unescape(raw: &[u8]) -> String {
    if !raw.contains(&b'&') {
        return String::from_utf8_lossy(raw).into_owned();
    }
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'&'
            && let Some(end) = raw[i..].iter().position(|&c| c == b';')
        {
            let ent = &raw[i + 1..i + end];
            let rep: Option<char> = match ent {
                b"amp" => Some('&'),
                b"lt" => Some('<'),
                b"gt" => Some('>'),
                b"quot" => Some('"'),
                b"apos" => Some('\''),
                _ if ent.first() == Some(&b'#') => {
                    let s = std::str::from_utf8(&ent[1..]).unwrap_or("");
                    let v = if let Some(h) = s.strip_prefix('x').or_else(|| s.strip_prefix('X')) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        s.parse::<u32>().ok()
                    };
                    v.and_then(char::from_u32)
                }
                _ => None,
            };
            if let Some(c) = rep {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                i += end + 1;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Escape text for inclusion in XML content or attribute values.
pub fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
}

/// Parse a complete XML document and return its root element.
pub fn parse(src: &[u8]) -> Result<Element, XmlError> {
    let mut p = Parser { src, pos: 0 };
    // Skip UTF-8 BOM
    if p.starts_with(&[0xEF, 0xBB, 0xBF]) {
        p.pos = 3;
    }
    p.skip_misc()?;
    p.element(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_doc() {
        let doc = br#"<?xml version="1.0"?><!-- c --><a x="1 &amp; 2"><b>t&lt;x&#65;</b><c/><![CDATA[<raw>]]></a>"#;
        let e = parse(doc).unwrap();
        assert_eq!(e.name, "a");
        assert_eq!(e.attr("x"), Some("1 & 2"));
        assert_eq!(e.child("b").unwrap().text(), "t<xA");
        assert!(e.child("c").unwrap().children.is_empty());
        assert_eq!(e.text(), "<raw>");
    }

    #[test]
    fn rejects_mismatch() {
        assert!(parse(b"<a><b></a>").is_err());
        assert!(parse(b"<a>").is_err());
    }
}
