//! SLURLs (LLSLURL): parse what is typed or pasted in the navigation bar,
//! build the SLURL of the current position, and label the place links of a
//! chat or profile text.
//!
//! Accepted: `http(s)://maps.secondlife.com/secondlife/Region/x/y/z`,
//! `slurl.com/secondlife/...`, `secondlife://Region/x/y/z`,
//! `secondlife:///app/teleport/Region/x/y/z`,
//! `secondlife:///secondlife/Region/x/y/z`, `Region/x/y/z`,
//! `Region (x, y, z)`, `Region, x, y, z` and a bare region name.

use glam::Vec3;

#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub region: String,
    pub pos: Vec3,
    /// Explicit grid authority of a hop / x-grid-location-info link.
    pub grid: Option<String>,
}

impl Location {
    pub fn target(&self) -> String {
        self.grid
            .as_ref()
            .map_or_else(|| self.region.clone(), |grid| crate::grids::target(grid, &self.region))
    }
}

/// The OpenSim grid the agent is on now (`grids::current`), set by the app
/// as it changes; None on Second Life. A global like the Legacy Viewer's
/// LFSimFeatureHandler, so the many place links need not carry it.
static GRID: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Name the grid the SLURLs made from now on are for (None: Second Life).
pub fn set_grid(grid: Option<String>) {
    if let Ok(mut current) = GRID.write()
        && *current != grid
    {
        *current = grid;
    }
}

thread_local! {
    /// RLVa `@showloc`: what a SLURL shows in place of a connected region,
    /// and the names of those regions (`set_hidden_regions`). Kept by the
    /// thread of the interface, which makes every SLURL a window shows; a
    /// test thread starts without it.
    static HIDDEN: std::cell::RefCell<Option<(String, Vec<String>)>> = const { std::cell::RefCell::new(None) };
}

/// RLVa `@showloc` from now on: `hidden` is the "(Hidden region)" label and
/// the names of the regions the agent is connected to; None without the
/// restriction. Set by the app every frame.
pub fn set_hidden_regions(hidden: Option<(String, Vec<String>)>) {
    HIDDEN.with(|h| {
        let mut h = h.borrow_mut();
        if *h != hidden {
            *h = hidden;
        }
    });
}

/// The SLURL of a place on the grid the agent is on, as a window shows or
/// copies it: on Second Life
/// `http://maps.secondlife.com/secondlife/Region%20Name/x/y/z`, on an OpenSim
/// grid `hop://grid.example:8002/Region%20Name/x/y/z`. Under RLVa `@showloc`
/// a region the agent is connected to is the "(Hidden region)" label, with
/// no position (`.../secondlife/(Hidden region)`), as
/// LLSLURL::getSLURLString does with RlvUtil::isNearbyRegion
/// (indra/newview/llslurl.cpp, rlvcommon.cpp: the exact region name).
pub fn make(region: &str, pos: Vec3) -> String {
    let grid = GRID.read().ok().and_then(|g| g.clone());
    let hidden = HIDDEN.with(|h| {
        h.borrow()
            .as_ref()
            .filter(|(_, regions)| regions.iter().any(|r| r == region))
            .map(|(label, _)| label.clone())
    });
    match hidden {
        Some(label) => hidden_on(grid.as_deref(), &label),
        None => make_on(grid.as_deref(), region, pos),
    }
}

/// The SLURL of a region RLVa `@showloc` hides: the grid's base then
/// `label` as it is, no position (LLGridManager::getSLURLBase +
/// RlvStrings::getString(Hidden::Region) in LLSLURL::getSLURLString).
pub fn hidden_on(grid: Option<&str>, label: &str) -> String {
    let base = make_on(grid, "", Vec3::ZERO);
    let base = base.strip_suffix("/0/0/0").unwrap_or(&base);
    format!("{base}{label}")
}

/// LLSLURL::getSLURLString: Second Life's maps address, or on an OpenSim
/// grid `hop://<grid>/` (DEFAULT_HOP_BASE, fsgridhandler.cpp), the grid
/// being the Hypergrid address of the region's grid (LLAgentUI::buildSLURL
/// uses LFSimFeatureHandler::hyperGridURL). Deliberate difference: the
/// Legacy Viewer gives the other place links the login grid, which is
/// wrong while visiting another grid; here every link names the grid the
/// agent is on, where its region was found.
pub fn make_on(grid: Option<&str>, region: &str, pos: Vec3) -> String {
    let (x, y, z) = (
        pos.x.round().max(0.0) as i32,
        pos.y.round().max(0.0) as i32,
        pos.z.round().max(0.0) as i32,
    );
    match grid.map(str::trim).filter(|g| !g.is_empty()) {
        Some(grid) => format!("hop://{}/{}/{x}/{y}/{z}", grid.trim_end_matches('/'), encode(region)),
        None => format!("http://maps.secondlife.com/secondlife/{}/{x}/{y}/{z}", encode(region)),
    }
}

pub fn encode(s: &str) -> String {
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
    if lower.starts_with("hop://") || lower.starts_with("x-grid-location-info://") {
        return parse_hypergrid(s);
    }
    // strip the known URL prefixes down to "Region/x/y/z"
    let mut rest: Option<&str> = None;
    for prefix in [
        "secondlife:///app/teleport/",
        "secondlife://app/teleport/",
        "secondlife:///app/region/",
        // LLSLURL: the path "secondlife" names a place of the main grid
        "secondlife:///secondlife/",
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
        return Some(Location { region, pos, grid: None });
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
                    grid: None,
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
                return Some(Location { region, pos, grid: None });
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
            grid: None,
        });
    }
    // bare region name: its center, on the ground
    Some(Location {
        region: s.to_owned(),
        pos: Vec3::new(128.0, 128.0, 0.0),
        grid: None,
    })
}

/// LLSLURL's OpenSim schemes. Keep the authority separate until the app
/// resolves it against the current grid and its known gatekeepers.
pub fn parse_hypergrid(input: &str) -> Option<Location> {
    let url = reqwest::Url::parse(input.trim()).ok()?;
    if !matches!(url.scheme(), "hop" | "x-grid-location-info") || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let address = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let grid = crate::grids::authority(&address)?;
    let path = url.path().trim_matches('/');
    let path = if url.scheme() == "x-grid-location-info" {
        path.strip_prefix("region/")
            .or_else(|| path.strip_prefix("secondlife/"))
            .unwrap_or(path)
    } else {
        path
    };
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 4 {
        return None;
    }
    let decoded = decode(&parts.first()?.replace('+', "%2B"));
    if decoded.chars().any(char::is_control) {
        return None;
    }
    let region = decoded.trim().to_owned();
    if region.is_empty() || region.contains(['/', '\\']) || region.chars().any(char::is_control) {
        return None;
    }
    let pos = coords(&parts[1..])?;
    Some(Location {
        region,
        pos,
        grid: Some(grid),
    })
}

/// A place link found in a chat or profile text, shown by its label instead
/// of the raw URL.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLink {
    pub location: Location,
    /// "Region (x,y,z)", or "Teleport me to Region (x,y,z)".
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
    if lower.starts_with("hop://") || lower.starts_with("x-grid-location-info://") {
        let location = parse_hypergrid(url)?;
        let label = format!(
            "{} ({:.0},{:.0},{:.0}) — {}",
            location.region,
            location.pos.x,
            location.pos.y,
            location.pos.z,
            location.grid.as_deref().unwrap_or_default()
        );
        return Some(PlaceLink {
            location,
            label,
            teleport: false,
        });
    }
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
        label = aurora_i18n::tr!("slurl-teleport-me-to", label = label.to_string());
    }
    Some(PlaceLink {
        location: Location {
            region,
            pos: coords(nums).unwrap_or(Vec3::new(128.0, 128.0, 0.0)),
            grid: None,
        },
        label,
        teleport,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn opensim_slurls_name_their_grid_and_read_back() {
        let pos = Vec3::new(128.4, 64.6, 22.0);
        assert_eq!(
            make_on(None, "Région 1", pos),
            "http://maps.secondlife.com/secondlife/R%C3%A9gion%201/128/65/22"
        );
        let hop = make_on(Some("grid.example:8002"), "Région 1", pos);
        assert_eq!(hop, "hop://grid.example:8002/R%C3%A9gion%201/128/65/22");
        let back = parse_hypergrid(&hop).expect("a hop SLURL reads back");
        assert_eq!(back.grid.as_deref(), Some("grid.example:8002"));
        assert_eq!(back.region, "Région 1");
        assert_eq!(back.pos, Vec3::new(128.0, 65.0, 22.0));
        // the navigation bar's parser keeps the grid too
        assert_eq!(parse(&hop).and_then(|l| l.grid).as_deref(), Some("grid.example:8002"));
        // an empty grid is Second Life's link
        assert!(make_on(Some(" "), "R", pos).starts_with("http://maps.secondlife.com/"));
    }

    use super::*;

    #[test]
    fn hypergrid_links_keep_the_grid_in_every_target() {
        for scheme in ["hop", "x-grid-location-info"] {
            let url = format!("{scheme}://grid.example:8002/Aurora%20D%C3%A9mo/12/34/56");
            let location = parse(&url).expect("Hypergrid location");
            assert_eq!(location.region, "Aurora Démo");
            assert_eq!(location.grid.as_deref(), Some("grid.example:8002"));
            assert_eq!(location.pos, Vec3::new(12.0, 34.0, 56.0));
            let target = parse_hypergrid(&location.target()).expect("context target");
            assert_eq!(target.region, location.region);
            assert_eq!(target.grid, location.grid);
            let place = place_link(&url).expect("clickable place");
            assert!(place.label.contains("grid.example:8002"));
            assert!(!place.teleport);
        }
        assert_eq!(parse_hypergrid("hop://grid.example/A+B").expect("plus is literal").region, "A+B");
        assert!(parse_hypergrid("hop://[::1]:9000/Test/1/2/3").is_some());
        for invalid in [
            "hop:///Region",
            "hop://user:secret@grid.example/Region",
            "hop://grid.example/Region/NaN/1/2",
            "hop://grid.example/Region/1/2/3/4",
            "hop://grid.example/%0ARegion",
            "hop://grid.example/%2FRegion",
        ] {
            assert!(parse_hypergrid(invalid).is_none(), "{invalid}");
        }
    }

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
        assert_eq!(l.label, aurora_i18n::tr!("slurl-teleport-me-to", label = "Ahern (1,2,3)").as_str());
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

#[cfg(test)]
mod rlva_tests {
    use super::*;

    /// LLSLURL::getSLURLString (indra/newview/llslurl.cpp): under `@showloc`
    /// a connected region is "(Hidden region)" with no position, another
    /// region keeps its address; the exact name counts.
    #[test]
    fn showloc_hides_the_connected_regions_of_a_slurl() {
        set_hidden_regions(Some(("(Hidden region)".into(), vec!["Aurora Démo".into()])));
        assert_eq!(
            make("Aurora Démo", Vec3::new(12.0, 34.0, 56.0)),
            "http://maps.secondlife.com/secondlife/(Hidden region)"
        );
        assert_eq!(
            make("Far Away", Vec3::new(1.0, 2.0, 3.0)),
            "http://maps.secondlife.com/secondlife/Far%20Away/1/2/3"
        );
        assert_eq!(
            make("aurora démo", Vec3::new(1.0, 2.0, 3.0)),
            "http://maps.secondlife.com/secondlife/aurora%20d%C3%A9mo/1/2/3"
        );
        assert_eq!(
            make_on(None, "Aurora Démo", Vec3::new(12.0, 34.0, 56.0)),
            "http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56"
        );
        assert_eq!(
            hidden_on(Some("grid.example:8002"), "(Hidden region)"),
            "hop://grid.example:8002/(Hidden region)"
        );
        set_hidden_regions(None);
        assert_eq!(
            make("Aurora Démo", Vec3::new(12.0, 34.0, 56.0)),
            "http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56"
        );
    }
}
