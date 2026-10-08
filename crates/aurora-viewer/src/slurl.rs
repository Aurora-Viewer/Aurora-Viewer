//! SLURLs (LLSLURL): parse what is typed or pasted in the navigation bar and
//! build the SLURL of the current position.
//!
//! Accepted: `http(s)://maps.secondlife.com/secondlife/Region/x/y/z`,
//! `slurl.com/secondlife/...`, `secondlife://Region/x/y/z`,
//! `secondlife:///app/teleport/Region/x/y/z`, `Region/x/y/z`,
//! `Region (x, y, z)`, `Region, x, y, z` and a bare region name.

use glam::Vec3;

#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub region: String,
    pub pos: Vec3,
}

/// `http://maps.secondlife.com/secondlife/Region%20Name/x/y/z`.
pub fn make(region: &str, pos: Vec3) -> String {
    format!(
        "http://maps.secondlife.com/secondlife/{}/{}/{}/{}",
        encode(region),
        pos.x.round().max(0.0) as i32,
        pos.y.round().max(0.0) as i32,
        pos.z.round().max(0.0) as i32
    )
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                // bytes, not str slices: never split a UTF-8 character
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn coords(parts: &[&str]) -> Option<Vec3> {
    let n: Vec<f32> = parts.iter().map(|p| p.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
    let v = |i: usize, d: f32| n.get(i).copied().unwrap_or(d);
    if n.iter().any(|x| !x.is_finite()) {
        return None;
    }
    // var regions go up to 8192 m; z up to the build limit
    Some(Vec3::new(
        v(0, 128.0).clamp(0.0, 8192.0),
        v(1, 128.0).clamp(0.0, 8192.0),
        v(2, 0.0).clamp(0.0, 4096.0),
    ))
}

/// Region and position from typed / pasted text.
pub fn parse(input: &str) -> Option<Location> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    // strip the known URL prefixes down to "Region/x/y/z"
    let mut rest: Option<&str> = None;
    for prefix in [
        "secondlife:///app/teleport/",
        "secondlife://app/teleport/",
        "secondlife:///app/region/",
        "secondlife://",
        "https://maps.secondlife.com/secondlife/",
        "http://maps.secondlife.com/secondlife/",
        "maps.secondlife.com/secondlife/",
        "https://slurl.com/secondlife/",
        "http://slurl.com/secondlife/",
        "slurl.com/secondlife/",
    ] {
        if lower.starts_with(prefix) {
            rest = Some(&s[prefix.len()..]);
            break;
        }
    }
    if let Some(r) = rest {
        let r = r.split(['?', '#']).next().unwrap_or("").trim_matches('/');
        let parts: Vec<&str> = r.split('/').collect();
        let region = decode(parts.first()?).trim().to_owned();
        if region.is_empty() {
            return None;
        }
        let pos = coords(&parts[1..parts.len().min(4)]).unwrap_or(Vec3::new(128.0, 128.0, 0.0));
        return Some(Location { region, pos });
    }
    // "Region (x, y, z)" as shown in the bar (optionally "Parcel, Region (...) - Maturity")
    if let (Some(open), Some(close)) = (s.rfind('('), s.rfind(')'))
        && open < close
    {
        let nums: Vec<&str> = s[open + 1..close].split(',').collect();
        if let Some(pos) = coords(&nums) {
            let head = s[..open].trim();
            let region = head.rsplit(", ").next().unwrap_or(head).trim();
            if !region.is_empty() {
                return Some(Location {
                    region: region.to_owned(),
                    pos,
                });
            }
        }
    }
    // "Region/x/y/z"
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if let Some(pos) = coords(&parts[1..parts.len().min(4)]) {
            let region = decode(parts[0]).trim().to_owned();
            if !region.is_empty() {
                return Some(Location { region, pos });
            }
        }
    }
    // "Region, x, y, z"
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() >= 3
        && let Some(pos) = coords(&parts[1..parts.len().min(4)])
    {
        return Some(Location {
            region: parts[0].trim().to_owned(),
            pos,
        });
    }
    // bare region name: its center, on the ground
    Some(Location {
        region: s.to_owned(),
        pos: Vec3::new(128.0, 128.0, 0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> (String, [i32; 3]) {
        let l = parse(s).expect(s);
        (l.region, [l.pos.x as i32, l.pos.y as i32, l.pos.z as i32])
    }

    #[test]
    fn formats() {
        let want = ("Aurora Démo".to_owned(), [12, 34, 56]);
        assert_eq!(p("http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56"), want);
        assert_eq!(
            p("https://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56/?title=x"),
            want
        );
        assert_eq!(p("secondlife://Aurora Démo/12/34/56"), want);
        assert_eq!(p("secondlife:///app/teleport/Aurora%20D%C3%A9mo/12/34/56"), want);
        assert_eq!(p("Aurora Démo/12/34/56"), want);
        assert_eq!(p("Aurora Démo (12, 34, 56)"), want);
        assert_eq!(
            p("Place d'Aurora, Aurora Démo (12, 34, 56) - Général"),
            ("Aurora Démo".to_owned(), [12, 34, 56])
        );
        assert_eq!(p("Aurora Démo, 12, 34, 56"), want);
        assert_eq!(p("Aurora Démo"), ("Aurora Démo".to_owned(), [128, 128, 0]));
        assert_eq!(
            p("http://maps.secondlife.com/secondlife/Ahern"),
            ("Ahern".to_owned(), [128, 128, 0])
        );
        assert!(parse("   ").is_none());
    }

    #[test]
    fn make_round_trips() {
        let s = make("Aurora Démo", Vec3::new(12.4, 34.6, 56.0));
        assert_eq!(s, "http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/35/56");
        assert_eq!(p(&s), ("Aurora Démo".to_owned(), [12, 35, 56]));
    }
}
