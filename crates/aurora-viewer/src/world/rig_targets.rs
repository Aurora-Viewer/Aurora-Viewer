//! Bounded link-name resolution. BuildTool routes individual property reads
//! without changing the user's Edit selection.
use super::objects::ObjKey;
use super::{
    World,
    hand_ik::{Seat, object_source},
    rig_graph::{Definition, ROOT_LINK, Target},
};
use aurora_net::{RegionHandle, build::ObjectProps};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_ROOTS: usize = 64;
const MAX_PRIMS: usize = 1024;
const MAX_LINKSET: usize = 256;
const QUERY_WINDOW: Duration = Duration::from_secs(1);
const MAX_QUERIES: usize = 4;
const RETRY: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Prim {
    name: Option<String>,
    attempts: u8,
    sent: Option<Instant>,
    fresh: bool,
}
#[derive(Clone, PartialEq, Eq)]
struct Requested {
    names: BTreeSet<String>,
    bundles: BTreeSet<(Uuid, Uuid)>,
    priority: u8,
}
impl Default for Requested {
    fn default() -> Self {
        Self {
            names: BTreeSet::new(),
            bundles: BTreeSet::new(),
            priority: 3,
        }
    }
}
#[derive(Default)]
struct Root {
    members: Vec<Uuid>,
    names: HashMap<String, Option<Uuid>>,
    complete: bool,
    requested: Requested,
    refreshed: bool,
}
#[derive(Default)]
pub struct Targets {
    region: Option<RegionHandle>,
    roots: HashMap<Uuid, Root>,
    prims: HashMap<Uuid, Prim>,
    queries: VecDeque<Instant>,
    changed: bool,
}

pub fn members(world: &World, root: Uuid) -> Option<Vec<Uuid>> {
    let source = object_source(world, root).filter(|s| s.root == root)?;
    let index = world.objects.index_of_uuid(&root)?;
    let object = world.objects.get(index)?;
    let mut result = vec![root];
    for &child in world.objects.children_of(&object.key) {
        let child = world.objects.get(child)?;
        if child.is_avatar() {
            continue;
        }
        if child.key.region != source.region || result.len() >= MAX_LINKSET {
            return None;
        }
        result.push(child.full_id);
    }
    result.sort_unstable();
    Some(result)
}

impl Targets {
    pub(crate) fn describe_link(&self, world: &World, source: Seat, object: Uuid, link: &str) -> String {
        let root = if object.is_nil() { source.root } else { object };
        if self.index(world, Some(source), object, link).is_some() {
            return "resolved".into();
        }
        if object_source(world, root).is_none() {
            return "object absent".into();
        }
        let Some(entry) = self.roots.get(&root) else {
            return "not requested".into();
        };
        let named = entry
            .members
            .iter()
            .filter(|id| self.prims.get(id).is_some_and(|p| p.name.is_some()))
            .count();
        let attempts: usize = entry
            .members
            .iter()
            .filter_map(|id| self.prims.get(id))
            .map(|p| p.attempts as usize)
            .sum();
        if !entry.complete {
            let missing: Vec<_> = entry
                .members
                .iter()
                .enumerate()
                .filter_map(|(index, id)| {
                    let prim = self.prims.get(id)?;
                    if prim.name.is_some() {
                        return None;
                    }
                    let role = if *id == root { "root".into() } else { format!("child[{index}]") };
                    Some(format!("{role}:{}", prim.attempts))
                })
                .take(8)
                .collect();
            return format!(
                "waiting for names {named}/{} attempts={attempts} missing={}{}",
                entry.members.len(),
                missing.join(","),
                if entry.members.len() - named > 8 { ",..." } else { "" }
            );
        }
        if entry.names.get(link) == Some(&None) {
            "duplicate name"
        } else {
            "name or parentage missing"
        }
        .into()
    }

    pub fn on_properties(&mut self, props: &[ObjectProps]) {
        for props in props {
            let Some(prim) = self.prims.get_mut(&props.object_id) else {
                continue;
            };
            let name = (props.name.len() <= 256).then_some(props.name.as_str());
            self.changed |= prim.fresh || prim.name.as_deref() != name;
            prim.name = name.map(str::to_owned);
            prim.fresh = false;
            if name.is_none() {
                prim.attempts = 3;
            }
        }
    }
    pub fn watches(&self, id: Uuid) -> bool {
        self.prims.contains_key(&id)
    }
    pub fn name(&self, id: Uuid) -> Option<&str> {
        self.prims.get(&id)?.name.as_deref()
    }
    pub fn index(&self, world: &World, source: Option<Seat>, object: Uuid, link: &str) -> Option<usize> {
        let root = if object.is_nil() { source?.root } else { object };
        let seat = object_source(world, root).filter(|s| s.root == root)?;
        if source.is_some_and(|s| s.region != seat.region) {
            return None;
        }
        if link == ROOT_LINK {
            return world.objects.index_of_uuid(&root);
        }
        let entry = self.roots.get(&root).filter(|r| r.complete)?;
        let id = entry.names.get(link).copied().flatten()?;
        let prim = self.prims.get(&id)?;
        if !prim.fresh && prim.name.as_deref() != Some(link) {
            return None;
        }
        // Stale maps cannot keep an unlinked/deleted child alive, even when
        // the render pass precedes the next metadata synchronization.
        super::hand_ik::target_index(world, seat, id)
    }

    fn sync(
        &mut self,
        world: &World,
        roots: &BTreeMap<Uuid, Requested>,
        props: &HashMap<Uuid, ObjectProps>,
        now: Instant,
    ) -> Option<ObjKey> {
        if world.teleporting || self.region != world.main_region {
            *self = Self {
                region: world.main_region,
                ..Self::default()
            };
        }
        if world.teleporting {
            return None;
        }
        let mut wanted = HashSet::new();
        let mut retained = HashSet::new();
        let mut planned = Vec::new();
        let mut candidates: Vec<_> = roots.iter().collect();
        candidates.sort_unstable_by_key(|(id, request)| (request.priority, !self.roots.contains_key(*id), **id));
        for (&id, requested) in candidates {
            if retained.len() >= MAX_ROOTS {
                break;
            }
            let Some(ids) = members(world, id) else { continue };
            if wanted.len() + ids.iter().filter(|id| !wanted.contains(*id)).count() > MAX_PRIMS {
                continue;
            }
            retained.insert(id);
            wanted.extend(ids.iter().copied());
            planned.push((id, ids, requested));
        }
        // Retire old metadata before admitting new roots, keeping the bound
        // even during a complete project/source switch.
        self.roots.retain(|id, _| retained.contains(id));
        self.prims.retain(|id, _| wanted.contains(id));
        let properties_changed = std::mem::take(&mut self.changed);
        for (id, ids, requested) in planned {
            let root = self.roots.entry(id).or_default();
            let topology = !root.members.is_empty() && root.members != ids;
            let mut changed = properties_changed || root.members != ids;
            if requested.names.iter().any(|name| !root.requested.names.contains(name))
                || requested.bundles.iter().any(|bundle| !root.requested.bundles.contains(bundle))
            {
                root.refreshed = false;
            }
            root.requested.clone_from(requested);
            if root.members != ids {
                root.members = ids;
            }
            for &id in &root.members {
                let prim = self.prims.entry(id).or_default();
                if prim.fresh {
                    continue;
                }
                if props.get(&id).is_some_and(|p| p.name.len() > 256) {
                    changed |= prim.name.take().is_some();
                    prim.attempts = 3;
                    continue;
                }
                if let Some(prop) = props.get(&id).filter(|p| p.name.len() <= 256)
                    && prim.name.as_deref() != Some(prop.name.as_str())
                {
                    prim.name = Some(prop.name.clone());
                    changed = true;
                }
            }
            if changed {
                let mut names = HashMap::new();
                let mut complete = true;
                for &id in &root.members {
                    let prim = self.prims.entry(id).or_default();
                    if let Some(name) = &prim.name {
                        names.entry(name.clone()).and_modify(|id| *id = None).or_insert(Some(id));
                    } else {
                        complete = false;
                    }
                }
                if complete || !root.complete {
                    root.names = names;
                    root.complete = complete;
                }
            }
            // Only topology or a newly declared unresolved name/bundle starts
            // a finite refresh. Repeated state/profile packets stay quiet.
            if topology
                || (!root.refreshed
                    && requested
                        .names
                        .iter()
                        .any(|name| !root.names.get(name).is_some_and(Option::is_some)))
            {
                root.refreshed = true;
                // A missing new name must not release unrelated, previously
                // verified contacts while their metadata is refreshed.
                if topology {
                    root.names.clear();
                    root.complete = false;
                }
                for id in &root.members {
                    self.prims.insert(
                        *id,
                        Prim {
                            fresh: true,
                            ..Default::default()
                        },
                    );
                }
            }
        }
        while self
            .queries
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= QUERY_WINDOW)
        {
            self.queries.pop_front();
        }
        // Small cold linksets can resolve on consecutive frames while every
        // trailing second still contains at most four property requests.
        if self.queries.len() >= MAX_QUERIES {
            return None;
        }
        let prims = &self.prims;
        let (_, _, _, id) =
            self.roots
                .iter()
                .flat_map(|(root, request)| {
                    request.members.iter().filter_map(move |id| {
                        let p = prims.get(id)?;
                        (p.name.is_none() && p.attempts < 3 && p.sent.is_none_or(|at| now.saturating_duration_since(at) >= RETRY))
                            .then_some((request.requested.priority, *root, p.attempts, *id))
                    })
                })
                .min()?;
        let object = world.objects.get(world.objects.index_of_uuid(&id)?)?;
        let prim = self.prims.get_mut(&id)?;
        prim.attempts += 1;
        prim.sent = Some(now);
        self.queries.push_back(now);
        Some(object.key)
    }
}

impl World {
    /// Include inactive declarations from occupied/worn sources to prepare
    /// their named contacts before an object switches its active profile.
    pub fn update_ik_targets(
        &mut self,
        editor: Option<(&Definition, Option<Uuid>)>,
        props: &HashMap<Uuid, ObjectProps>,
        now: Instant,
    ) -> Option<ObjKey> {
        let mut roots: BTreeMap<Uuid, Requested> = BTreeMap::new();
        let mut add = |definition: &Definition, source: Option<Uuid>, bundle: Uuid, priority: u8| {
            for target in definition.targets() {
                if let Target::Prim { id, name } = target
                    && let Some(root) = object_source(self, *id).filter(|s| s.root != *id).map(|s| s.root)
                {
                    let requested = roots.entry(root).or_default();
                    requested.priority = requested.priority.min(priority);
                    if !name.is_empty() {
                        requested.names.insert(name.clone());
                    }
                    requested.bundles.insert((source.unwrap_or_default(), bundle));
                }
                if let Target::Link { object, link } = target
                    && link != ROOT_LINK
                    && let Some(root) = if object.is_nil() { source } else { Some(*object) }
                {
                    let requested = roots.entry(root).or_default();
                    requested.priority = requested.priority.min(priority);
                    requested.names.insert(link.clone());
                    requested.bundles.insert((source.unwrap_or_default(), bundle));
                }
            }
        };
        self.hand_contacts.for_each_definition(&mut |definition, source, bundle| {
            let priority = object_source(self, source).map_or(3, |seat| {
                if super::hand_ik::own_seat(self) == Some(seat) {
                    1
                } else if super::hand_ik::source_eligible_for(self, self.agent_id, seat) {
                    2
                } else {
                    3
                }
            });
            add(definition, Some(source), bundle, priority);
        });
        if let Some((definition, source)) = editor {
            add(definition, source, Uuid::nil(), 0);
        }
        let mut targets = std::mem::take(&mut self.ik_targets);
        let result = targets.sync(self, &roots, props, now);
        self.ik_targets = targets;
        result
    }
}

#[cfg(test)]
mod tests;
