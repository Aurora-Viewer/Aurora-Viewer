//! SLURLs (LLSLURL): parse what is typed or pasted in the navigation bar,
//! build the SLURL of the current position, and label the place links of a
//! chat or profile text.
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

/// A place link found in a chat or profile text, shown by its label instead
/// of the raw URL.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLink {
    pub location: Location,
    /// "Region (x,y,z)", or "Me téléporter vers Region (x,y,z)".
    pub label: String,
    /// `secondlife:///app/teleport/...`: a click teleports (once confirmed).
    pub teleport: bool,
}

/// Recognize a place URL and build its label, like Firestorm's
/// LLUrlEntrySLURL (maps.secondlife.com / slurl.com), LLUrlEntryPlace
/// (`secondlife://Region/x/y[/z]`), LLUrlEntryRegion
/// (`secondlife:///app/region/...`) and LLUrlEntryTeleport
/// (`secondlife:///app/teleport/...`), indra/llui/llurlentry.cpp.
pub fn place_link(url: &str) -> Option<PlaceLink> {
    let lower = url.to_ascii_lowercase();
    let after = |prefix: &str| lower.starts_with(prefix).then(|| &url[prefix.len()..]);
    // (rest, teleport link, minimum number of coordinates)
    let (rest, teleport, min) = if let Some(r) = [
        "https://maps.secondlife.com/secondlife/",
        "http://maps.secondlife.com/secondlife/",
        "https://slurl.com/secondlife/",
        "http://slurl.com/secondlife/",
        "secondlife:///app/region/",
    ]
    .iter()
    .find_map(|p| after(p))
    {
        (r, false, 0)
    } else if let Some(r) = after("secondlife:///app/teleport/") {
        (r, true, 0)
    } else if lower.starts_with("secondlife:///") {
        // other app links (agent, group, inventory...)
        return None;
    } else {
        (after("secondlife://")?, false, 2)
    };
    let path = rest.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    let region = decode(parts[0]).trim().to_owned();
    let nums = &parts[1..];
    if region.is_empty() || nums.len() < min || nums.len() > 3 || nums.iter().any(|n| n.parse::<i32>().is_err()) {
        return None;
    }
    let mut label = region.clone();
    if !nums.is_empty() {
        label = format!("{label} ({})", nums.join(","));
    }
    if teleport {
        label = format!("Me téléporter vers {label}");
    }
    Some(PlaceLink {
        location: Location {
            region,
            pos: coords(nums).unwrap_or(Vec3::new(128.0, 128.0, 0.0)),
        },
        label,
        teleport,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_links() {
        let l = place_link("http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56").expect("slurl");
        assert_eq!(l.label, "Aurora Démo (12,34,56)");
        assert_eq!(l.location.pos, Vec3::new(12.0, 34.0, 56.0));
        assert!(!l.teleport);
        let l = place_link("https://maps.secondlife.com/secondlife/Ahern/50/60/?title=Hi").expect("slurl");
        assert_eq!(l.label, "Ahern (50,60)");
        assert_eq!(
            place_link("https://slurl.com/secondlife/Ahern").map(|l| l.label).as_deref(),
            Some("Ahern")
        );
        assert_eq!(
            place_link("secondlife://Ahern/1/2/3").map(|l| l.label).as_deref(),
            Some("Ahern (1,2,3)")
        );
        assert_eq!(
            place_link("secondlife:///app/region/Ahern/1/2/3").map(|l| l.label).as_deref(),
            Some("Ahern (1,2,3)")
        );
        let l = place_link("secondlife:///app/teleport/Ahern/1/2/3").expect("teleport");
        assert_eq!(l.label, "Me téléporter vers Ahern (1,2,3)");
        assert!(l.teleport);
        // not places
        assert!(place_link("secondlife:///app/agent/0e346d8b-4433-4d66-a6b0-fd37083abc4c/about").is_none());
        assert!(place_link("secondlife://Ahern").is_none());
        assert!(place_link("https://maps.secondlife.com/secondlife/Ahern/x/y").is_none());
        assert!(place_link("https://example.com/secondlife/Ahern/1/2/3").is_none());
    }

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
