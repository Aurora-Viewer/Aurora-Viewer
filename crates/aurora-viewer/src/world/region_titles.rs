//! The group title worn by region: a region given a group and a title once,
//! and the viewer wears them on every arrival there (teleport, crossing,
//! login). Port of FSGroupTitleRegionMgr (indra/newview/
//! fsgrouptitleregionmgr.cpp, originally LGPL 2.1):
//! - the rules are kept by region name, trimmed, spaces collapsed, control
//!   characters dropped, 63 bytes at most and compared without case
//!   (sanitizeRegionName, normalizeRegionName); one title per region, a
//!   title may have several regions;
//! - on a region change (gAgent's region changed callback, here the end of
//!   the arrival once the region's name and our groups are known), the
//!   region's rule is applied once (mLastAppliedRegion): GroupTitleUpdate
//!   unless that title is already the selected one, then ActivateGroup
//!   unless the group is already the active one (applyGroupTitle). A rule
//!   whose group we left is skipped, as is everything while RLVa's
//!   @setgroup holds the group;
//! - a region without a rule wears the default title when "Use default
//!   title in regions without assignment" is on (none_on_unassigned,
//!   default_group_id / default_role_id), else nothing changes;
//! - a typed region name is looked up on the map (sendNamedRegionRequest)
//!   and kept under its own spelling, or refused after 10 s
//!   (REGION_VALIDATION_TIMEOUT, FSSetTitleRegionNotFound).
//!
//! The titles of every group are asked here for the Group titles window
//! (FSFloaterGroupTitles::refreshGroupTitles), apart from the group
//! profiles' cache (`group_data.rs`), which keeps 20 groups only.
//!
//! The rules live in the account's folder, one file per grid (the Legacy
//! Viewer's `region_group_titles.xml` in the per-account folder). Unlike it,
//! a rule also keeps the title's and group's names: a rule whose group was
//! left is still listed by name, marked as ignored.

use aurora_net::NetCommand;
use aurora_net::groups::GroupTitle;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// MAX_REGION_NAME_LENGTH.
pub const MAX_REGION_NAME: usize = 63;
/// REGION_VALIDATION_TIMEOUT: a typed name the map did not answer by then
/// is not a region.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The account's file on one grid (`grid` from `chat_log::grid_name`).
pub fn file_name(grid: &str) -> String {
    format!("region_group_titles-{grid}.json")
}

/// sanitizeRegionName: trimmed, runs of spaces made one, control characters
/// dropped, cut at 63 bytes (on a character boundary).
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut space = false;
    for c in input.trim().chars() {
        if c == ' ' {
            if !space {
                out.push(c);
            }
            space = true;
        } else if !c.is_control() {
            out.push(c);
            space = false;
        }
    }
    if out.len() > MAX_REGION_NAME {
        let mut end = MAX_REGION_NAME;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out.trim_end().to_owned()
}

/// normalizeRegionName: the key a region name is compared by.
pub fn normalize(input: &str) -> String {
    sanitize(input).to_lowercase()
}

/// What a region wears: a group (nil: no group) and one of its titles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Target {
    pub group: Uuid,
    pub role: Uuid,
    /// The title's and group's names when the rule was made, shown once
    /// the group is no longer in our list.
    pub title: String,
    pub group_name: String,
}

impl Target {
    pub fn same(&self, group: Uuid, role: Uuid) -> bool {
        self.group == group && (group.is_nil() || self.role == role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The region's name as the map spells it (or as typed, sanitized).
    pub region: String,
    #[serde(flatten)]
    pub target: Target,
}

/// The account's rules on one grid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub rules: Vec<Rule>,
    /// none_on_unassigned: the default title in regions without a rule.
    pub use_default: bool,
    pub default: Target,
}

impl Saved {
    pub fn rule_for(&self, region: &str) -> Option<&Rule> {
        let key = normalize(region);
        self.rules.iter().find(|r| normalize(&r.region) == key)
    }

    /// setAssignment: the region's rule replaced; false when it already
    /// wore that title (or the name is empty).
    pub fn assign(&mut self, region: &str, target: Target) -> bool {
        let region = sanitize(region);
        if region.is_empty() {
            return false;
        }
        let key = normalize(&region);
        if let Some(r) = self.rules.iter_mut().find(|r| normalize(&r.region) == key) {
            if r.target.same(target.group, target.role) {
                return false;
            }
            r.region = region;
            r.target = target;
        } else {
            self.rules.push(Rule { region, target });
            self.rules.sort_by_cached_key(|r| normalize(&r.region));
        }
        true
    }

    /// clearAssignmentByRegion.
    pub fn remove(&mut self, region: &str) -> bool {
        let key = normalize(region);
        let before = self.rules.len();
        self.rules.retain(|r| normalize(&r.region) != key);
        self.rules.len() != before
    }

    /// clearAssignment: every region of a title.
    pub fn remove_title(&mut self, group: Uuid, role: Uuid) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| !r.target.same(group, role));
        self.rules.len() != before
    }

    /// getRegionDisplayNamesForTitle.
    pub fn regions_of(&self, group: Uuid, role: Uuid) -> Vec<&str> {
        self.rules
            .iter()
            .filter(|r| r.target.same(group, role))
            .map(|r| r.region.as_str())
            .collect()
    }

    /// The names a rule keeps, brought up to date from the titles known
    /// (a title renamed by the group's officers).
    fn refresh_names(&mut self, group: Uuid, group_name: &str, titles: &[GroupTitle]) -> bool {
        let mut changed = false;
        let targets = self
            .rules
            .iter_mut()
            .map(|r| &mut r.target)
            .chain(std::iter::once(&mut self.default));
        for t in targets.filter(|t| t.group == group && !group.is_nil()) {
            if let Some(title) = titles.iter().find(|x| x.role == t.role)
                && (t.title != title.title || t.group_name != group_name)
            {
                t.title = title.title.clone();
                t.group_name = group_name.to_owned();
                changed = true;
            }
        }
        changed
    }
}

/// What we wear now, as far as the viewer knows.
#[derive(Debug, Clone, Copy)]
pub struct Wearing<'a> {
    /// AgentDataUpdate's active group (nil: none) and its title.
    pub active_group: Uuid,
    pub active_title: &'a str,
    /// The selected title of the rule's group, when its titles are known.
    pub selected_role: Option<Uuid>,
}

/// What an arrival asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No rule for the region, no default title.
    NoRule,
    /// The rule's group is not one of ours any more.
    GroupLeft,
    /// That group and title are already worn: nothing is sent.
    AlreadyWorn,
    /// GroupTitleUpdate (group, role) then ActivateGroup, each when needed.
    Wear {
        title: Option<(Uuid, Uuid)>,
        activate: Option<Uuid>,
    },
}

impl Decision {
    /// The messages, in applyGroupTitle's order.
    pub fn commands(&self) -> Vec<NetCommand> {
        let Decision::Wear { title, activate } = self else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some((group, role)) = title {
            out.push(NetCommand::GroupTitleUpdate {
                group: *group,
                role: *role,
            });
        }
        if let Some(group) = activate {
            out.push(NetCommand::ActivateGroup(*group));
        }
        out
    }
}

/// applyGroupTitle for `target`. The title is taken as worn when the
/// group's titles say it is selected or, when they are not known, when the
/// active group's title reads the same (AgentDataUpdate carries no role).
pub fn wear(target: &Target, member: bool, now: Wearing) -> Decision {
    if !target.group.is_nil() && !member {
        return Decision::GroupLeft;
    }
    let title_worn = target.group.is_nil()
        || match now.selected_role {
            Some(role) => role == target.role,
            None => now.active_group == target.group && now.active_title == target.title,
        };
    let title = (!title_worn).then_some((target.group, target.role));
    let activate = (now.active_group != target.group).then_some(target.group);
    if title.is_none() && activate.is_none() {
        Decision::AlreadyWorn
    } else {
        Decision::Wear { title, activate }
    }
}

/// onRegionChanged's choice for `region`: its rule, else the default title
/// when it is on.
pub fn decide<'a>(saved: &Saved, region: &str, member: impl Fn(&Uuid) -> bool, now: impl Fn(&Target) -> Wearing<'a>) -> Decision {
    let target = match saved.rule_for(region) {
        Some(rule) => &rule.target,
        None if saved.use_default => &saved.default,
        None => return Decision::NoRule,
    };
    wear(target, member(&target.group), now(target))
}

/// A typed region name waiting for the map's answer.
#[derive(Debug, Clone)]
pub struct Lookup {
    pub name: String,
    pub target: Target,
    pub since: Instant,
}

/// The result of a lookup, for the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupResult {
    Added(String),
    NotFound(String),
}

#[derive(Default)]
pub struct RegionTitles {
    pub saved: Saved,
    path: Option<PathBuf>,
    /// The region whose arrival was handled (mLastAppliedRegion).
    handled: Option<String>,
    /// The region handle whose name was read: the frames after it skip
    /// the name until the agent's region changes.
    seen: Option<aurora_net::RegionHandle>,
    /// The titles of each of our groups, for the window.
    pub titles: HashMap<Uuid, Vec<GroupTitle>>,
    /// The request each group's titles answer.
    requests: HashMap<Uuid, Uuid>,
    pub lookup: Option<Lookup>,
    pub lookup_result: Option<LookupResult>,
    pub out: Vec<NetCommand>,
}

impl RegionTitles {
    /// loadFromDisk; a file that cannot be read starts empty.
    pub fn load(path: PathBuf) -> Self {
        let saved = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Saved>(&bytes).unwrap_or_else(|e| {
                log::warn!("region group titles unreadable ({e}): {}", path.display());
                Saved::default()
            }),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("region group titles not read ({e}): {}", path.display());
                }
                Saved::default()
            }
        };
        log::info!("region group titles: {} rule(s)", saved.rules.len());
        RegionTitles {
            saved,
            path: Some(path),
            ..Default::default()
        }
    }

    /// In memory only (the demo).
    pub fn in_memory(saved: Saved) -> Self {
        RegionTitles {
            saved,
            ..Default::default()
        }
    }

    /// saveToDisk, after every change.
    pub fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(&self.saved) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    log::warn!("region group titles not saved ({e}): {}", path.display());
                }
            }
            Err(e) => log::warn!("region group titles not encoded: {e}"),
        }
    }

    pub fn assign(&mut self, region: &str, target: Target) {
        if self.saved.assign(region, target) {
            log::info!("region group titles: {} assigned", sanitize(region));
            self.save();
        }
    }

    pub fn remove(&mut self, region: &str) {
        if self.saved.remove(region) {
            self.save();
        }
    }

    pub fn remove_title(&mut self, group: Uuid, role: Uuid) {
        if self.saved.remove_title(group, role) {
            self.save();
        }
    }

    /// setEnabled.
    pub fn set_use_default(&mut self, on: bool) {
        if self.saved.use_default != on {
            self.saved.use_default = on;
            self.save();
        }
    }

    /// onSetAsDefault: the title marked, and the default switched on.
    pub fn set_default(&mut self, target: Target) {
        if self.saved.default != target || !self.saved.use_default {
            self.saved.default = target;
            self.saved.use_default = true;
            self.save();
        }
    }

    /// The arrival in `region` was handled: true the first time for that
    /// region since the last change of region.
    pub fn arrived(&mut self, region: &str) -> bool {
        let key = normalize(region);
        if key.is_empty() || self.handled.as_deref() == Some(key.as_str()) {
            return false;
        }
        self.handled = Some(key);
        true
    }

    /// sendGroupTitlesRequest for each group not asked yet.
    pub fn request_titles(&mut self, groups: impl IntoIterator<Item = Uuid>) {
        for group in groups {
            if group.is_nil() || self.requests.contains_key(&group) {
                continue;
            }
            let request = Uuid::new_v4();
            self.requests.insert(group, request);
            self.titles.remove(&group);
            self.out.push(NetCommand::GroupTitlesRequest { group, request });
        }
    }

    /// FSFloaterGroupTitles::refreshGroupTitles: every list asked again.
    pub fn refresh_titles(&mut self, groups: impl IntoIterator<Item = Uuid>) {
        self.requests.clear();
        self.titles.clear();
        self.request_titles(groups);
    }

    /// GroupTitlesReply to one of our requests: true when it was ours.
    pub fn on_titles(&mut self, group: Uuid, request: Uuid, titles: &[GroupTitle], group_name: &str) -> bool {
        if self.requests.get(&group) != Some(&request) {
            return false;
        }
        self.titles.entry(group).or_default().extend(titles.iter().cloned());
        let all = self.titles.get(&group).cloned().unwrap_or_default();
        if self.saved.refresh_names(group, group_name, &all) {
            self.save();
        }
        true
    }

    /// A title chosen (here or elsewhere): our copy follows at once.
    pub fn title_sent(&mut self, group: Uuid, role: Uuid) {
        if let Some(list) = self.titles.get_mut(&group) {
            for t in list {
                t.selected = t.role == role;
            }
        }
    }

    /// The selected title of `group`, when its list is known.
    pub fn selected_role(&self, group: &Uuid) -> Option<Uuid> {
        self.titles.get(group)?.iter().find(|t| t.selected).map(|t| t.role)
    }

    /// validateAndSetAssignment: a name known to the map is assigned at
    /// once; another one is asked (MapNameRequest) and waits.
    pub fn look_up(&mut self, typed: &str, target: Target, known: Option<String>, now: Instant) {
        let name = sanitize(typed);
        self.lookup_result = None;
        if name.is_empty() {
            return;
        }
        if let Some(canonical) = known {
            self.lookup = None;
            self.assign(&canonical, target);
            self.lookup_result = Some(LookupResult::Added(canonical));
            return;
        }
        self.out.push(NetCommand::MapNameRequest { name: name.to_lowercase() });
        self.lookup = Some(Lookup { name, target, since: now });
    }

    /// onValidationResult / onValidationTimeout: `found` is the map's
    /// spelling of the name once it answered.
    pub fn lookup_tick(&mut self, found: Option<String>, now: Instant) {
        let Some(lookup) = &self.lookup else {
            return;
        };
        if let Some(canonical) = found {
            let target = lookup.target.clone();
            self.lookup = None;
            self.assign(&canonical, target);
            self.lookup_result = Some(LookupResult::Added(canonical));
        } else if now.duration_since(lookup.since) >= LOOKUP_TIMEOUT {
            log::info!("region group titles: no region named {:?}", lookup.name);
            self.lookup_result = Some(LookupResult::NotFound(lookup.name.clone()));
            self.lookup = None;
        }
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

impl super::World {
    /// The map's spelling of a region name, when it knows the region.
    pub fn map_region_named(&self, name: &str) -> Option<String> {
        let key = normalize(name);
        self.map
            .sims
            .values()
            .find(|s| !s.name.is_empty() && normalize(&s.name) == key)
            .map(|s| s.name.clone())
    }

    /// The arrival's title, once per region: the commands to send. Waits
    /// for the end of the arrival, the region's name and our groups
    /// (AgentGroupDataUpdate and AgentDataUpdate) before deciding.
    pub fn region_title_tick(&mut self, now: Instant) -> Vec<NetCommand> {
        if let Some(lookup) = &self.region_titles.lookup {
            let found = self.map_region_named(&lookup.name);
            self.region_titles.lookup_tick(found, now);
        }
        let mut out = self.region_titles.take_commands();
        if !self.movement_complete || !self.groups.list_received || !self.groups.active_received {
            return out;
        }
        if self.region_titles.seen.is_some() && self.region_titles.seen == self.main_region {
            return out;
        }
        let region = self.region_name();
        if region.is_empty() {
            return out;
        }
        self.region_titles.seen = self.main_region;
        if !self.region_titles.arrived(&region) {
            return out;
        }
        let groups = &self.groups;
        let titles = &self.region_titles;
        let data = &self.group_data;
        let decision = decide(
            &titles.saved,
            &region,
            |g| groups.is_member(g),
            |t| Wearing {
                active_group: groups.active,
                active_title: &groups.active_title,
                selected_role: titles
                    .selected_role(&t.group)
                    .or_else(|| data.get(&t.group).and_then(|d| d.selected_title()).map(|s| s.role)),
            },
        );
        let commands = decision.commands();
        if commands.is_empty() {
            log::info!("region group title: {decision:?} in {region}");
            return out;
        }
        // RlvActions::canChangeActiveGroup: nothing while @setgroup holds it
        if commands.iter().any(|c| !self.rlva_allows_command(c, Uuid::nil())) {
            log::info!("region group title: held by RLVa in {region}");
            return out;
        }
        log::info!("region group title: {decision:?} in {region}");
        out.extend(commands);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G1: Uuid = Uuid::from_u128(1);
    const G2: Uuid = Uuid::from_u128(2);
    const R1: Uuid = Uuid::from_u128(11);
    const R2: Uuid = Uuid::from_u128(12);

    fn target(group: Uuid, role: Uuid, title: &str) -> Target {
        Target {
            group,
            role,
            title: title.into(),
            group_name: "Group".into(),
        }
    }

    fn wearing(group: Uuid, title: &str, selected: Option<Uuid>) -> Wearing<'_> {
        Wearing {
            active_group: group,
            active_title: title,
            selected_role: selected,
        }
    }

    #[test]
    fn region_names_are_sanitized_and_compared_without_case() {
        assert_eq!(sanitize("  Some   Region\t "), "Some Region");
        assert_eq!(normalize("Some REGION"), "some region");
        let long = "é".repeat(40);
        let cut = sanitize(&long);
        assert!(cut.len() <= MAX_REGION_NAME && cut.chars().all(|c| c == 'é'));
        let mut s = Saved::default();
        assert!(s.assign("Some Region", target(G1, R1, "Builder")));
        assert!(s.rule_for("  some   region ").is_some());
        assert!(!s.assign("some region", target(G1, R1, "Builder")), "the same title again");
        assert_eq!(s.rules.len(), 1);
        assert!(!s.assign("   ", target(G1, R1, "Builder")));
    }

    #[test]
    fn one_title_per_region_and_several_regions_per_title() {
        let mut s = Saved::default();
        s.assign("North", target(G1, R1, "Builder"));
        s.assign("South", target(G1, R1, "Builder"));
        assert!(!s.assign("North", target(G1, R1, "Builder")), "unchanged");
        assert!(s.assign("NORTH", target(G2, R2, "Guide")), "changed");
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.rule_for("north").map(|r| r.target.group), Some(G2));
        assert_eq!(s.regions_of(G1, R1), vec!["South"]);
        s.assign("East", target(G1, R1, "Builder"));
        assert!(s.remove_title(G1, R1));
        assert_eq!(s.rules.len(), 1);
        assert!(s.remove("North"));
        assert!(!s.remove("North"));
    }

    #[test]
    fn nothing_without_a_rule_unless_the_default_is_on() {
        let mut s = Saved::default();
        s.assign("North", target(G1, R1, "Builder"));
        let d = decide(&s, "Elsewhere", |_| true, |_| wearing(G2, "Guide", None));
        assert_eq!(d, Decision::NoRule);
        assert!(d.commands().is_empty());
        s.use_default = true;
        s.default = Target::default();
        let d = decide(&s, "Elsewhere", |_| true, |_| wearing(G2, "Guide", None));
        assert_eq!(
            d,
            Decision::Wear {
                title: None,
                activate: Some(Uuid::nil())
            },
            "the default <no group title> activates no group"
        );
    }

    #[test]
    fn nothing_is_sent_when_the_title_is_already_worn() {
        let mut s = Saved::default();
        s.assign("North", target(G1, R1, "Builder"));
        assert_eq!(
            decide(&s, "North", |_| true, |_| wearing(G1, "Builder", None)),
            Decision::AlreadyWorn
        );
        assert_eq!(
            decide(&s, "North", |_| true, |_| wearing(G1, "Other", Some(R1))),
            Decision::AlreadyWorn
        );
        s.assign("Plain", Target::default());
        assert_eq!(
            decide(&s, "Plain", |_| true, |_| wearing(Uuid::nil(), "", None)),
            Decision::AlreadyWorn
        );
    }

    #[test]
    fn the_title_then_the_group_are_sent_when_needed() {
        let mut s = Saved::default();
        s.assign("North", target(G1, R1, "Builder"));
        // another group active, the title not selected in ours
        let d = decide(&s, "North", |_| true, |_| wearing(G2, "Guide", Some(R2)));
        assert!(matches!(
            &d.commands()[..],
            [NetCommand::GroupTitleUpdate { group: G1, role: R1 }, NetCommand::ActivateGroup(G1)]
        ));
        // the group active with another title
        let d = decide(&s, "North", |_| true, |_| wearing(G1, "Guide", Some(R2)));
        assert!(matches!(&d.commands()[..], [NetCommand::GroupTitleUpdate { group: G1, role: R1 }]));
        // the title already selected in the group, the group not active
        let d = decide(&s, "North", |_| true, |_| wearing(G2, "Guide", Some(R1)));
        assert!(matches!(&d.commands()[..], [NetCommand::ActivateGroup(G1)]));
        // titles unknown and another group active: the title is sent too
        let d = decide(&s, "North", |_| true, |_| wearing(G2, "Builder", None));
        assert_eq!(d.commands().len(), 2);
    }

    #[test]
    fn a_rule_for_a_group_left_is_ignored() {
        let mut s = Saved::default();
        s.assign("North", target(G1, R1, "Builder"));
        let d = decide(&s, "North", |g| *g != G1, |_| wearing(G2, "Guide", None));
        assert_eq!(d, Decision::GroupLeft);
        assert!(d.commands().is_empty());
    }

    #[test]
    fn each_region_is_handled_once_until_the_next_one() {
        let mut t = RegionTitles::default();
        assert!(t.arrived("North"));
        assert!(!t.arrived("north"), "the same region again: the user's own choice stays");
        assert!(t.arrived("South"));
        assert!(t.arrived("North"));
        assert!(!t.arrived(""));
    }

    #[test]
    fn titles_are_ours_by_request_and_rename_the_rules() {
        let mut t = RegionTitles::default();
        t.saved.assign("North", target(G1, R1, "Old"));
        t.request_titles([G1, G1, Uuid::nil()]);
        let sent = t.take_commands();
        let [NetCommand::GroupTitlesRequest { group: G1, request }] = sent[..] else {
            panic!("one request: {sent:?}");
        };
        let titles = [GroupTitle {
            title: "New".into(),
            role: R1,
            selected: true,
        }];
        assert!(!t.on_titles(G1, Uuid::new_v4(), &titles, "Group"), "not ours");
        assert!(t.on_titles(G1, request, &titles, "Renamed group"));
        assert_eq!(t.saved.rules[0].target.title, "New");
        assert_eq!(t.saved.rules[0].target.group_name, "Renamed group");
        assert_eq!(t.selected_role(&G1), Some(R1));
        t.title_sent(G1, R2);
        assert_eq!(t.selected_role(&G1), None);
    }

    #[test]
    fn a_typed_region_is_kept_under_the_maps_spelling_or_refused() {
        let start = Instant::now();
        let mut t = RegionTitles::default();
        t.look_up("  boreal   lagoon ", target(G1, R1, "Builder"), None, start);
        assert!(matches!(&t.take_commands()[..], [NetCommand::MapNameRequest { name }] if name == "boreal lagoon"));
        t.lookup_tick(None, start + Duration::from_secs(1));
        assert!(t.lookup.is_some());
        t.lookup_tick(Some("Boreal Lagoon".into()), start + Duration::from_secs(2));
        assert_eq!(t.saved.rules[0].region, "Boreal Lagoon");
        assert_eq!(t.lookup_result, Some(LookupResult::Added("Boreal Lagoon".into())));
        t.look_up("Nowhere", target(G1, R1, "Builder"), None, start);
        t.take_commands();
        t.lookup_tick(None, start + LOOKUP_TIMEOUT);
        assert_eq!(t.lookup_result, Some(LookupResult::NotFound("Nowhere".into())));
        assert_eq!(t.saved.rules.len(), 1);
        t.look_up("Known", target(G2, R2, "Guide"), Some("KNOWN".into()), start);
        assert!(t.take_commands().is_empty() && t.saved.rule_for("known").is_some());
    }

    /// The arrival waits for the move, the region's name and our groups,
    /// sends once per region, and nothing while RLVa holds the group.
    #[test]
    fn the_arrival_waits_for_the_region_and_the_groups() {
        use aurora_net::{GroupMembership, NetEvent};
        let mut w = super::super::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.region_titles.saved.assign("North", target(G1, R1, "Builder"));
        let (north, south): (u64, u64) = (1 << 40, 2 << 40);
        w.set_main_region(north);
        let now = Instant::now();
        assert!(w.region_title_tick(now).is_empty(), "the move is not complete");
        w.movement_complete = true;
        assert!(w.region_title_tick(now).is_empty(), "no name, no groups yet");
        w.ensure_region(north, (256, 256)).name = "North".into();
        assert!(w.region_title_tick(now).is_empty(), "our groups are not known yet");
        let member = |id| GroupMembership {
            id,
            name: "Group".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        };
        w.apply(NetEvent::Groups(vec![member(G1), member(G2)]));
        w.apply(NetEvent::ActiveGroup {
            id: G2,
            name: "Group".into(),
            title: "Guide".into(),
        });
        let sent = w.region_title_tick(now);
        assert!(
            matches!(
                &sent[..],
                [NetCommand::GroupTitleUpdate { group: G1, role: R1 }, NetCommand::ActivateGroup(G1)]
            ),
            "{sent:?}"
        );
        assert!(w.region_title_tick(now).is_empty(), "once per region");
        // the user's own choice stays: nothing for a region without a rule
        w.set_main_region(south);
        w.ensure_region(south, (256, 256)).name = "South".into();
        assert!(w.region_title_tick(now).is_empty());
        // back in North, under @setgroup: nothing either
        w.rlva.configure(true);
        assert!(w.rlva.receive(&aurora_net::ChatMessage {
            from_name: "HUD".into(),
            source_id: G2,
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: "@setgroup=n".into(),
        }));
        w.set_main_region(north);
        assert!(w.region_title_tick(now).is_empty(), "held by RLVa");
    }

    #[test]
    fn the_rules_survive_a_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("aurora-region-titles-{}", Uuid::new_v4()));
        let path = dir.join(file_name("agni"));
        let mut t = RegionTitles::load(path.clone());
        t.assign("North", target(G1, R1, "Builder"));
        t.set_default(target(G2, R2, "Guide"));
        let back = RegionTitles::load(path);
        assert_eq!(back.saved, t.saved);
        assert!(back.saved.use_default);
        let _ = std::fs::remove_dir_all(dir);
    }
}
