//! Saved Area Search defaults and column visibility. Filter and link semantics
//! follow FSAreaSearch::matchObject / applySlappParams / buildSlappLink
//! (indra/newview/fsareasearch.cpp; originally LGPL 2.1), with Aurora's precise
//! distances, live quick search and broader default results preserved.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Filters {
    pub query: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub creator: String,
    pub group: String,
    pub last_owner: String,
    pub regex: bool,
    pub locked: bool,
    pub copy: bool,
    pub modify: bool,
    pub transfer: bool,
    pub media: bool,
    pub probe: bool,
    pub include_attachments: bool,
    pub only_attachments: bool,
    pub exclude_physical: bool,
    pub exclude_temporary: bool,
    pub exclude_probes: bool,
    pub min_price: i32,
    pub max_price: i32,
    pub min_distance: f32,
    /// Legacy combo index: 0 unrestricted, 1 any, 2 touch, 3..9 click codes 1..7.
    pub click_action: u8,
    pub parcel_only: bool,
    pub children: bool,
    pub neighbors: bool,
    pub mine: bool,
    pub sale: bool,
    pub physical: bool,
    pub phantom: bool,
    pub temporary: bool,
    pub distance_on: bool,
    pub max_distance: f32,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            query: String::new(),
            name: String::new(),
            description: String::new(),
            owner: String::new(),
            creator: String::new(),
            group: String::new(),
            last_owner: String::new(),
            regex: false,
            locked: false,
            copy: false,
            modify: false,
            transfer: false,
            media: false,
            probe: false,
            include_attachments: false,
            only_attachments: false,
            exclude_physical: false,
            exclude_temporary: false,
            exclude_probes: false,
            min_price: 0,
            max_price: 999999,
            min_distance: 0.0,
            click_action: 0,
            parcel_only: false,
            children: false,
            neighbors: false,
            mine: false,
            sale: false,
            physical: false,
            phantom: false,
            temporary: false,
            distance_on: false,
            max_distance: 96.0,
        }
    }
}

impl Filters {
    /// Exclusions win, as FSPanelAreaSearchFilter::onCommitCheckbox.
    pub fn normalize(&mut self) {
        if !self.include_attachments {
            self.only_attachments = false;
        }
        if self.exclude_physical {
            self.physical = false;
        }
        if self.exclude_temporary {
            self.temporary = false;
        }
        if self.exclude_probes {
            self.probe = false;
        }
        self.click_action = self.click_action.min(9);
        self.min_price = self.min_price.clamp(0, 999999999);
        self.max_price = self.max_price.clamp(self.min_price, 999999999);
        self.min_distance = if self.min_distance.is_finite() {
            self.min_distance.clamp(0.0, 999999999.0)
        } else {
            0.0
        };
        self.max_distance = if self.max_distance.is_finite() {
            self.max_distance.clamp(self.min_distance, 999999999.0)
        } else {
            999999.0
        };
        for text in [
            &mut self.query,
            &mut self.name,
            &mut self.description,
            &mut self.owner,
            &mut self.creator,
            &mut self.group,
            &mut self.last_owner,
        ] {
            if let Some((end, _)) = text.char_indices().nth(256) {
                text.truncate(end);
            }
        }
    }
    /// Save filter rules, never the current search text (Legacy Save as Default).
    pub fn without_search(&self) -> Self {
        let mut f = self.clone();
        f.query.clear();
        f.name.clear();
        f.description.clear();
        f.owner.clear();
        f.creator.clear();
        f.group.clear();
        f.last_owner.clear();
        f.regex = false;
        f.normalize();
        f
    }
}

pub const DEFAULT_COLUMNS: [bool; 10] = [true, true, true, true, true, false, true, false, false, false];

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AreaSearch {
    pub filters: Filters,
    pub columns: [bool; 10],
}
impl Default for AreaSearch {
    fn default() -> Self {
        Self {
            filters: Filters::default(),
            columns: DEFAULT_COLUMNS,
        }
    }
}
impl AreaSearch {
    pub fn normalize(&mut self) {
        self.filters.normalize();
        if !self.columns.iter().any(|on| *on) {
            self.columns[1] = true;
        }
    }
}
const CLICK_NAMES: [&str; 10] = ["blank", "any", "touch", "sit", "buy", "pay", "open", "play", "open_media", "zoom"];

impl Filters {
    /// FSAreaSearch::buildSlappLink: Legacy parameter names and percent escaping.
    /// q / mine preserve Aurora's additional quick query and ownership shortcut.
    pub fn link(&self) -> String {
        let mut f = self.clone();
        f.normalize();
        let mut args = Vec::new();
        let mut add = |key: &str, value: String| args.push(format!("{key}={}", crate::slurl::encode(&value)));
        for (key, text) in [
            ("nm", &f.name),
            ("ds", &f.description),
            ("ow", &f.owner),
            ("gr", &f.group),
            ("cr", &f.creator),
            ("lo", &f.last_owner),
            ("q", &f.query),
        ] {
            if !text.is_empty() {
                add(key, text.clone());
            }
        }
        add("fs", u8::from(f.sale).to_string());
        add("fd", u8::from(f.distance_on).to_string());
        for (key, flag) in [
            ("rx", f.regex),
            ("fl", f.locked),
            ("fp", f.physical),
            ("ft", f.temporary),
            ("fh", f.phantom),
            ("fa", f.only_attachments),
            ("fm", f.media),
            ("fr", f.probe),
            ("pc", f.copy),
            ("pm", f.modify),
            ("pt", f.transfer),
            ("ap", f.parcel_only),
            ("xa", !f.include_attachments),
            ("xp", f.exclude_physical),
            ("xt", f.exclude_temporary),
            ("xr", f.exclude_probes),
            ("xc", !f.children),
            ("xn", !f.neighbors),
            ("mine", f.mine),
        ] {
            if flag {
                add(key, "1".into());
            }
        }
        add("smn", f.min_price.to_string());
        add("smx", f.max_price.to_string());
        add("dmn", f.min_distance.to_string());
        add("dmx", f.max_distance.to_string());
        if f.click_action > 0 {
            add("ca", CLICK_NAMES[usize::from(f.click_action)].into());
        }
        let mut url = "secondlife:///app/openfloater/area_search".to_owned();
        if !args.is_empty() {
            url.push('?');
            url.push_str(&args.join("&"));
        }
        url
    }

    /// FSAreaSearch::applySlappParams: omitted booleans are false, independent of
    /// saved defaults. Unknown keys are ignored; imports never overwrite defaults.
    pub fn from_query(query: &str) -> Option<Self> {
        if query.len() > 32768 {
            return None;
        }
        let mut f = Self::default();
        let value = |key| crate::slapp::query_value(query, key);
        for (key, text) in [
            ("nm", &mut f.name),
            ("ds", &mut f.description),
            ("ow", &mut f.owner),
            ("gr", &mut f.group),
            ("cr", &mut f.creator),
            ("lo", &mut f.last_owner),
            ("q", &mut f.query),
        ] {
            *text = value(key).unwrap_or_default();
        }
        let flag = |key| value(key).is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"));
        for (key, target) in [
            ("rx", &mut f.regex),
            ("fl", &mut f.locked),
            ("fp", &mut f.physical),
            ("ft", &mut f.temporary),
            ("fh", &mut f.phantom),
            ("fa", &mut f.only_attachments),
            ("fm", &mut f.media),
            ("fr", &mut f.probe),
            ("pc", &mut f.copy),
            ("pm", &mut f.modify),
            ("pt", &mut f.transfer),
            ("ap", &mut f.parcel_only),
            ("xp", &mut f.exclude_physical),
            ("xt", &mut f.exclude_temporary),
            ("xr", &mut f.exclude_probes),
            ("mine", &mut f.mine),
        ] {
            *target = flag(key);
        }
        f.include_attachments = !flag("xa");
        f.children = !flag("xc");
        f.neighbors = !flag("xn");
        let number = |key, default| {
            value(key)
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(default)
                .clamp(0, 999999999) as i32
        };
        f.min_price = number("smn", 0);
        f.max_price = number("smx", 999999);
        f.sale = if value("fs").is_some() {
            flag("fs")
        } else {
            value("smn").is_some() || value("smx").is_some()
        };
        let distance = |key, default| {
            value(key)
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|n| n.is_finite())
                .unwrap_or(default)
                .clamp(0.0, 999999999.0)
        };
        f.min_distance = distance("dmn", 0.0);
        f.max_distance = distance("dmx", 999999.0);
        f.distance_on = if value("fd").is_some() {
            flag("fd")
        } else {
            value("dmn").is_some() || value("dmx").is_some()
        };
        f.click_action = value("ca").and_then(|s| CLICK_NAMES.iter().position(|n| *n == s)).unwrap_or(0) as u8;
        f.normalize();
        Some(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slapp::{AppLink, parse};

    #[test]
    fn area_search_links_round_trip_every_option_and_unicode_without_enabling_dormant_ranges() {
        for click_action in 0..=9 {
            for (sale, distance_on) in [(false, false), (true, true)] {
                let f = Filters {
                    name: "Chair & café + 座席".into(),
                    description: r"(?i)quiet.*".into(),
                    owner: "Owner".into(),
                    group: "Group".into(),
                    creator: "Creator".into(),
                    last_owner: "Last".into(),
                    query: "quick".into(),
                    regex: true,
                    locked: true,
                    physical: true,
                    phantom: true,
                    temporary: true,
                    include_attachments: true,
                    only_attachments: true,
                    media: true,
                    probe: true,
                    copy: true,
                    modify: true,
                    transfer: true,
                    children: true,
                    neighbors: true,
                    parcel_only: true,
                    mine: true,
                    min_price: 13,
                    max_price: 700,
                    min_distance: 2.5,
                    max_distance: 87.5,
                    sale,
                    distance_on,
                    click_action,
                    ..Default::default()
                };
                let Some(AppLink::AreaSearch(restored)) = parse(&f.link()) else {
                    panic!("search link not routed");
                };
                assert_eq!(*restored, f, "action {click_action}, sale {sale}");
            }
        }
        let f = Filters::default();
        assert_eq!(parse(&f.link()), Some(AppLink::AreaSearch(Box::new(f))));
    }

    #[test]
    fn area_search_legacy_links_bound_inputs_and_do_not_inherit_saved_exclusions() {
        let f = Filters::from_query("nm=Chair%2BSeat&fp=YES&xa=1&fa=1&xp=on&smn=-2&smx=2000000000&dmn=4&dmx=6&ca=open_media").unwrap();
        assert_eq!(f.name, "Chair+Seat");
        assert!(!f.physical && f.exclude_physical);
        assert!(!f.only_attachments && !f.include_attachments);
        assert!(f.children && f.neighbors, "omitted exclusion keys mean false");
        assert!(f.sale && f.distance_on);
        assert_eq!((f.min_price, f.max_price), (0, 999999999));
        assert_eq!((f.min_distance, f.max_distance), (4.0, 6.0));
        assert_eq!(f.click_action, 8);
        assert_eq!(Filters::from_query("dmn=NaN&dmx=inf&ca=invalid").unwrap().max_distance, 999999.0);
        let long = format!("nm={}", "語".repeat(300));
        assert_eq!(Filters::from_query(&long).unwrap().name.chars().count(), 256);
        assert!(Filters::from_query(&"x".repeat(32769)).is_none());
        assert!(matches!(
            parse("secondlife:///app/openfloater/unrelated?fp=1"),
            Some(AppLink::Unsupported)
        ));
    }

    #[test]
    fn area_search_settings_round_trip_and_old_files_keep_aurora_defaults() {
        let old: AreaSearch = serde_json::from_str("{}").unwrap();
        assert_eq!(old, AreaSearch::default());
        assert!(
            !old.filters.exclude_physical && !old.filters.exclude_temporary,
            "retain Aurora's broader results"
        );
        let mut saved = AreaSearch {
            columns: [false; 10],
            filters: Filters {
                query: "private search".into(),
                name: "chair".into(),
                regex: true,
                physical: true,
                exclude_physical: true,
                only_attachments: true,
                min_distance: f32::NAN,
                ..Default::default()
            },
        };
        saved.normalize();
        assert!(saved.columns[1]);
        assert!(!saved.filters.physical && !saved.filters.only_attachments);
        saved.filters = saved.filters.without_search();
        assert!(saved.filters.name.is_empty() && saved.filters.query.is_empty() && !saved.filters.regex);
        let encoded = serde_json::to_string(&saved).unwrap();
        assert!(!encoded.contains("private"));
        assert_eq!(serde_json::from_str::<AreaSearch>(&encoded).unwrap(), saved);
    }
}
