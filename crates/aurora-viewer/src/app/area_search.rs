//! Area Search's live data and bounded ObjectSelect/ObjectDeselect requests.
//! FSAreaSearch::findObjects / processRequestQueue / requestObjectProperties
//! (indra/newview/fsareasearch.cpp). Avatars are excluded; attachments are opt-in.
use super::*;
use crate::ui::area_search::{Action, Row};
use crate::world::objects::{ObjKey, Object, ObjectStore};
use aurora_net::build::{BuildCmd, ObjectProps};
use rayon::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use uuid::Uuid;

const PACKET: usize = 254;
const IN_FLIGHT: usize = PACKET * 3;
const TIMEOUT: Duration = Duration::from_secs(30);
/// Between two touches of the menu's selection (FSAreaSearchTouchTimer).
const TOUCH_SPACING: Duration = Duration::from_millis(200);

#[derive(Default)]
pub(super) struct Feed {
    /// Touches waiting for their turn: when, which object.
    touches: VecDeque<(Instant, Uuid)>,
    props: HashMap<Uuid, ObjectProps>,
    costs: HashMap<Uuid, f32>,
    cost_pending: HashMap<u64, (Instant, Vec<Uuid>)>,
    cost_asked: HashMap<Uuid, Instant>,
    requests: HashMap<Uuid, (ObjKey, Instant)>,
    recent: HashMap<Uuid, Instant>,
    last_request: Option<Instant>,
    next: Option<Instant>,
    region: Option<aurora_net::RegionHandle>,
    seeded: bool,
    scope: (bool, bool, bool, bool, bool, bool, bool),
}

impl Feed {
    pub(super) fn expects(&self, id: &Uuid) -> bool {
        self.recent.get(id).is_some_and(|until| Instant::now() < *until)
    }

    fn reset(&mut self) {
        // Replies and reliable duplicates can arrive after closing or crossing.
        // Preserve their provenance and outstanding counts through the timeout.
        // Touches already asked for are sent too, as the Legacy Viewer's timers.
        let requests = std::mem::take(&mut self.requests);
        let recent = std::mem::take(&mut self.recent);
        let touches = std::mem::take(&mut self.touches);
        let last_request = self.last_request;
        let cost_pending = std::mem::take(&mut self.cost_pending);
        let cost_asked = std::mem::take(&mut self.cost_asked);
        *self = Self {
            touches,
            requests,
            recent,
            last_request,
            cost_pending,
            cost_asked,
            ..Default::default()
        };
    }

    pub(super) fn on_closed_properties(&mut self, props: &[ObjectProps]) {
        for p in props.iter().filter(|p| p.full) {
            self.requests.remove(&p.object_id);
        }
    }

    pub(super) fn on_properties(&mut self, props: &[ObjectProps]) {
        for p in props.iter().filter(|p| p.full) {
            self.requests.remove(&p.object_id);
            self.props.insert(p.object_id, p.clone());
        }
    }

    /// At most three 64-object capability requests in flight. Failed or partial
    /// replies wait the regular timeout before retrying; Refresh cannot bypass it.
    fn cost_batches(&mut self, candidates: &[(Uuid, ObjKey)], now: Instant) -> HashMap<aurora_net::RegionHandle, Vec<Uuid>> {
        self.cost_pending.retain(|_, (at, _)| now.saturating_duration_since(*at) < TIMEOUT);
        self.cost_asked.retain(|_, at| now.saturating_duration_since(*at) < TIMEOUT);
        let count: usize = self.cost_pending.values().map(|(_, ids)| ids.len()).sum();
        let mut remaining = 192usize.saturating_sub(count);
        let mut batches = HashMap::<_, Vec<_>>::new();
        // One batch per snapshot limits both HTTP traffic and frame-thread work.
        for &(id, key) in candidates {
            if remaining == 0 {
                break;
            }
            if self.costs.contains_key(&id) || self.cost_asked.contains_key(&id) {
                continue;
            }
            if !batches.is_empty() && !batches.contains_key(&key.region) {
                continue;
            }
            let batch = batches.entry(key.region).or_default();
            if batch.len() == 64 {
                break;
            }
            batch.push(id);
            self.cost_asked.insert(id, now);
            remaining -= 1;
        }
        batches
    }

    pub(super) fn on_cost_reply(&mut self, tag: u64, result: &Result<aurora_llsd::Llsd, String>) {
        let Some((_, ids)) = self.cost_pending.remove(&tag) else {
            return;
        };
        if !self.seeded {
            return;
        }
        let Ok(reply) = result else {
            return;
        };
        for id in ids {
            let object = &reply[id.to_string().as_str()];
            let cost = match &object["linked_set_resource_cost"] {
                aurora_llsd::Llsd::Integer(n) => *n as f32,
                aurora_llsd::Llsd::Real(n) => *n as f32,
                _ => continue,
            };
            if cost.is_finite() && cost >= 0.0 {
                self.costs.insert(id, cost);
                self.cost_asked.remove(&id);
            }
        }
        self.next = None;
    }
    fn requests(&mut self, candidates: &[(Uuid, ObjKey)], now: Instant) -> HashMap<aurora_net::RegionHandle, Vec<u32>> {
        self.recent.retain(|_, until| now < *until);
        // A scope change cannot cancel a packet already sent. Keep counting it
        // until its reply or timeout, even if its row is now outside the list.
        self.requests
            .retain(|id, (_, sent)| now.saturating_duration_since(*sent) < TIMEOUT && !self.props.contains_key(id));
        let mut counts = HashMap::<_, usize>::new();
        for (key, _) in self.requests.values() {
            *counts.entry(key.region).or_default() += 1;
        }
        let mut packets = HashMap::<_, Vec<u32>>::new();
        for &(id, key) in candidates {
            if self.props.contains_key(&id) || self.requests.contains_key(&id) {
                continue;
            }
            let count = counts.entry(key.region).or_default();
            let packet = packets.entry(key.region).or_default();
            if *count >= IN_FLIGHT || packet.len() >= PACKET {
                continue;
            }
            packet.push(key.local_id);
            *count += 1;
            self.requests.insert(id, (key, now));
            self.recent.insert(id, now + TIMEOUT);
            self.last_request = Some(now);
        }
        packets.retain(|_, ids| !ids.is_empty());
        packets
    }
}

/// Validated ancestry, before any metadata request. Attachment roots remain
/// roots even though their parent is an avatar (isSearchableObject/root-edit).
struct Placement {
    position: Vec3,
    attachment: bool,
    child: bool,
    wearer: Option<usize>,
}
fn placement(store: &ObjectStore, original: &Object) -> Option<Placement> {
    if original.is_avatar() || original.is_client_preview() || original.full_id.is_nil() {
        return None;
    }
    let mut object = original;
    let mut pos = object.position;
    for depth in 0..64 {
        if object.parent_id == 0 {
            return pos.is_finite().then_some(Placement {
                position: pos,
                attachment: original.is_attachment(),
                child: original.parent_id != 0,
                wearer: None,
            });
        }
        let parent = store.parent_of(object)?;
        object = store.get(parent)?;
        if object.is_client_preview() {
            return None;
        }
        if object.is_avatar() {
            // The live snapshot resolves the bone-anchored world transform.
            return Some(Placement {
                position: object.position,
                attachment: true,
                child: depth != 0,
                wearer: Some(parent),
            });
        }
        pos = object.position + object.rotation * pos;
    }
    None
}
impl App {
    pub(super) fn area_search_build_properties(&self, props: Vec<ObjectProps>, area_reads: &HashSet<Uuid>) -> Vec<ObjectProps> {
        if area_reads.is_empty() {
            return props;
        }
        let used: HashSet<_> = self
            .build
            .highlighted(&self.world)
            .into_iter()
            .chain(self.inspect_prims())
            .filter_map(|(i, _)| self.world.objects.get(i).map(|o| o.full_id))
            .collect();
        for_build(props, area_reads, &self.build.props, &used)
    }

    pub(super) fn demo_area_search(&mut self, mode: &str) {
        self.panels.area_search = true;
        self.panels.people = false;
        self.panels.chat = false;
        self.panels.minimap = false;
        self.area_search_ui.table.filters = Default::default();
        self.area_search_ui.demo_controls(mode);
        self.area_search_ui.demo_menu = mode == "menu";
        self.area_search_ui.table.filters.parcel_only = mode == "parcel";
        self.area_search_ui.table.filters.sale = mode == "sale";
        if mode == "search" {
            self.area_search_ui.table.filters.query = aurora_i18n::tr!("demo-fountain").into();
        }
        let keys: Vec<_> = self
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.parent_id == 0 && !o.is_avatar())
            .map(|(_, o)| o.key)
            .take(8)
            .collect();
        for (n, key) in keys.into_iter().enumerate() {
            let name = match n % 4 {
                0 => aurora_i18n::tr!("demo-fountain"),
                1 => aurora_i18n::tr!("demo-game-table"),
                2 => aurora_i18n::tr!("demo-cube-for-sale"),
                _ => aurora_i18n::tr!("demo-spare-cube"),
            };
            self.send(NetCommand::Build(BuildCmd::SetName {
                handle: key.region,
                local_id: key.local_id,
                name: name.into(),
            }));
            self.send(NetCommand::Build(BuildCmd::SetDescription {
                handle: key.region,
                local_id: key.local_id,
                description: aurora_i18n::tr!("area-search-demo-description").into(),
            }));
            if n % 2 == 0 {
                self.send(NetCommand::Build(BuildCmd::SetSaleInfo {
                    handle: key.region,
                    local_ids: vec![key.local_id],
                    sale_type: 1,
                    price: (n as i32 + 1) * 25,
                }));
            }
        }
    }

    pub(super) fn area_search_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        // touches already asked for go on once the window is closed
        self.area_search_touches(Instant::now());
        if !self.panels.area_search {
            if self.area_search_feed.seeded {
                self.area_search_feed.reset();
                self.area_search_ui.table.replace(Vec::new());
                self.area_search_ui.selection.clear();
            }
            if self
                .area_search_feed
                .last_request
                .is_some_and(|t| Instant::now().saturating_duration_since(t) >= TIMEOUT)
            {
                self.area_search_feed.requests.clear();
                self.area_search_feed.recent.clear();
                self.area_search_feed.last_request = None;
            }
            return;
        }
        let now = Instant::now();
        let refresh = std::mem::take(&mut self.area_search_ui.refresh);
        if self.area_search_feed.region != self.world.main_region {
            self.area_search_feed.reset();
            self.area_search_feed.region = self.world.main_region;
            self.area_search_ui.selection.clear();
        }
        if !self.area_search_feed.seeded {
            self.area_search_feed
                .on_properties(&self.build.props.values().cloned().collect::<Vec<_>>());
            self.area_search_feed.seeded = true;
        }
        if refresh {
            self.area_search_feed.props.clear();
            self.area_search_feed.costs.clear();
            // Keep outstanding requests: repeated Refresh must not bypass the cap.
            self.area_search_feed.next = None;
        }
        let filters = &self.area_search_ui.table.filters;
        let scope = (
            filters.children,
            filters.parcel_only,
            filters.neighbors,
            filters.include_attachments,
            filters.exclude_physical,
            filters.exclude_temporary,
            filters.exclude_probes,
        );
        if self.area_search_feed.scope != scope {
            self.area_search_feed.scope = scope;
            self.area_search_feed.next = None;
        }
        if self.area_search_feed.next.is_none_or(|t| now >= t) {
            self.area_search_feed.next = Some(now + Duration::from_millis(500));
            self.area_search_snapshot(now);
        }
        let parcel_known = self.world.parcel.as_ref().is_some_and(|p| !p.bitmap.is_empty());
        let draw_distance = self.settings.draw_distance;
        if let Some(action) = self
            .area_search_ui
            .show(ctx, p, &mut self.panels.area_search, parcel_known, draw_distance)
        {
            self.area_search_action(action, now);
        }
        let columns = self.area_search_ui.saved_columns();
        let defaults = std::mem::take(&mut self.area_search_ui.save_defaults);
        if defaults || self.settings.area_search.columns != columns {
            self.settings.area_search.columns = columns;
            if defaults {
                self.settings.area_search.filters = self.area_search_ui.table.filters.without_search();
            }
            self.settings.save();
        }
    }

    /// FSPanelAreaSearchList::onContextMenuItemClick, through the actions
    /// of the object menus (their confirmations and RLVa checks included).
    fn area_search_action(&mut self, action: Action, now: Instant) {
        use ui::context::CtxAction;
        let position = |app: &Self, id: &Uuid| {
            app.world
                .objects
                .index_of_uuid(id)
                .and_then(|idx| Scene::object_transform(&app.world, idx, now, 0))
                .map(|xf| xf.0)
        };
        match action {
            Action::Inspect(ids) => self.inspect_objects(&ids),
            Action::Zoom(id) => {
                if let Some(pos) = position(self, &id) {
                    self.on_ctx_action(CtxAction::Zoom(pos, None));
                }
            }
            Action::ScriptInfo(ids) => {
                for id in ids {
                    self.on_ctx_action(CtxAction::ScriptInfo(id));
                }
            }
            // FSAreaSearchTouchTimer: one every 0.2 s, in the selection's order
            Action::Touch(ids) => {
                let start = self
                    .area_search_feed
                    .touches
                    .back()
                    .map_or(now, |(t, _)| (*t + TOUCH_SPACING).max(now));
                for (n, id) in ids.into_iter().enumerate() {
                    self.area_search_feed.touches.push_back((start + TOUCH_SPACING * n as u32, id));
                }
            }
            // sitOnObject: AgentRequestSit with no offset
            Action::Sit(id) => self.on_ctx_action(CtxAction::Sit {
                target: id,
                offset: Vec3::ZERO,
            }),
            // gAgent.teleportViaLocation(getPositionGlobal())
            Action::Teleport(id) => {
                if let Some(pos) = position(self, &id) {
                    self.teleport_render_pos(pos);
                }
            }
            Action::Edit(ids) => {
                self.area_search_select(&ids);
                if !self.build.selection.is_empty() {
                    self.build.open_build(crate::build::Tool::Edit);
                    self.build.edit_mode = crate::build::EditMode::Move;
                }
                self.flush_build();
            }
            Action::Return(ids) => self.ask_return_objects(&ids),
            Action::Delete(ids) => {
                // LLSelectMgr::selectDelete's question, on all their linksets
                let flags: Vec<u32> = ids
                    .iter()
                    .filter_map(|id| self.world.objects.index_of_uuid(id))
                    .flat_map(|idx| crate::build::family(&self.world, context_menu::linkset_root(&self.world, idx)))
                    .filter_map(|i| self.world.objects.get(i).map(|o| o.update_flags))
                    .collect();
                match ui::context::delete_warning(&flags) {
                    Some(text) => self.delete_confirm = Some((ids, text)),
                    None => self.delete_objects(&ids),
                }
            }
            Action::Blacklist(ids) => {
                for id in ids {
                    self.derender(id, true);
                }
            }
            Action::Buy(id) => self.on_ctx_action(CtxAction::Buy(id)),
            // setFindOwnerText(gAgentUsername), then the search
            Action::FilterMine => {
                let name = self.world.social.name_of(&self.world.agent_id);
                self.area_search_ui.table.filters.owner = name;
                self.area_search_ui.reveal_search();
            }
        }
    }

    fn area_search_select(&mut self, ids: &[Uuid]) {
        let idxs: Vec<usize> = ids.iter().filter_map(|id| self.world.objects.index_of_uuid(id)).collect();
        self.build.select_many(&mut self.world, &self.settings.build, &idxs);
    }

    /// Delete to the trash, confirmed: the objects of several rows together.
    pub(super) fn delete_objects(&mut self, ids: &[Uuid]) {
        self.area_search_select(ids);
        self.build.delete(&mut self.world);
        self.flush_build();
    }

    /// FSPanelAreaSearchList::touchObject when its turn comes: ObjectGrab
    /// then ObjectDeGrab with an empty pick (LLPickInfo's defaults).
    fn area_search_touches(&mut self, now: Instant) {
        while let Some(&(at, id)) = self.area_search_feed.touches.front() {
            if at > now {
                break;
            }
            self.area_search_feed.touches.pop_front();
            let Some(key) = self
                .world
                .objects
                .index_of_uuid(&id)
                .and_then(|idx| self.world.objects.get(idx))
                .map(|o| o.key)
            else {
                continue;
            };
            let surface = aurora_net::TouchSurface::default();
            self.send(NetCommand::ObjectGrab {
                handle: key.region,
                local_id: key.local_id,
                offset: Vec3::ZERO,
                surface,
            });
            self.send(NetCommand::ObjectRelease {
                handle: key.region,
                local_id: key.local_id,
                surface,
            });
        }
    }

    fn area_search_snapshot(&mut self, now: Instant) {
        self.area_search_feed
            .props
            .retain(|id, _| self.world.objects.index_of_uuid(id).is_some());
        let world = &self.world;
        let props = &self.area_search_feed.props;
        self.area_search_feed
            .costs
            .retain(|id, _| self.world.objects.index_of_uuid(id).is_some());
        let area_costs = &self.area_search_feed.costs;
        let store = &world.objects;
        let agent = world.agent_global().unwrap_or_default();
        let origin = world.main_origin().unwrap_or_default();
        let bitmap = world.parcel.as_ref().map(|p| p.bitmap.as_slice()).unwrap_or_default();
        let regions: HashSet<_> = world.regions.keys().copied().collect();
        let filters = &self.area_search_ui.table.filters;
        let children = filters.children;
        let parcel_only = self.area_search_ui.table.filters.parcel_only;
        let neighbors = self.area_search_ui.table.filters.neighbors;
        let region = world.main_region;
        let rows: Vec<_> = store
            .slots
            .par_iter()
            .enumerate()
            .filter_map(|(index, o)| {
                let o = o.as_ref()?;
                if !regions.contains(&o.key.region) || (!neighbors && Some(o.key.region) != region) {
                    return None;
                }
                let place = placement(store, o)?;
                if (!children && place.child)
                    || (!filters.include_attachments && place.attachment)
                    || (filters.exclude_physical && o.update_flags & 1 != 0)
                    || (filters.exclude_temporary && o.update_flags & (1 << 29) != 0)
                    || (filters.exclude_probes && o.extra.reflection_probe.is_some())
                {
                    return None;
                }
                let pos = if place.attachment {
                    let transform = Scene::object_transform(world, index, now, 0)?;
                    let position = if transform.2 {
                        // HUD coordinates are screen-space; their useful world
                        // distance and parcel are those of their wearer.
                        Scene::object_transform(world, place.wearer?, now, 0)?.0
                    } else {
                        transform.0
                    };
                    position - world.region_offset(o.key.region)?
                } else {
                    place.position
                };
                let (x, y) = aurora_net::handle_to_origin(o.key.region);
                let global = glam::DVec3::new(f64::from(x), f64::from(y), 0.0) + pos.as_dvec3();
                let on_parcel = crate::world::landmarks::in_parcel(bitmap, origin, global);
                if parcel_only && !on_parcel {
                    return None;
                }
                let mut row = Row::default();
                row.id = o.full_id;
                row.distance = global.distance(agent) as f32;
                row.on_parcel = on_parcel;
                row.child = place.child;
                row.attachment = place.attachment;
                row.flags = o.update_flags;
                row.click_action = o.click_action;
                row.probe = o.extra.reflection_probe.is_some();
                row.media =
                    o.te.as_ref()
                        .is_some_and(|te| te.faces.iter().any(|face| face.media_flags & 1 != 0));
                row.prim_count = store.children_of(&o.key).len() as u32 + 1;
                row.land_impact = area_costs.get(&o.full_id).copied();
                row.mine = o.update_flags & ui::context::flags::OBJECT_YOU_OWNER != 0;
                if let Some(p) = props.get(&o.full_id) {
                    row.loaded = true;
                    row.name.clone_from(&p.name);
                    row.description.clone_from(&p.description);
                    row.owner_id = p.owner_id;
                    row.creator_id = p.creator_id;
                    row.last_owner_id = p.last_owner_id;
                    row.owner_mask = p.owner_mask;
                    row.group_id = p.group_id;
                    if row.owner_id.is_nil() && !row.group_id.is_nil() {
                        row.owner_id = row.group_id;
                    }
                    row.price = (p.sale_type != 0).then_some(p.sale_price);
                }
                Some((row, o.key))
            })
            .collect();
        let candidates: Vec<_> = rows.iter().map(|(r, key)| (r.id, *key)).collect();
        let packets = self.area_search_feed.requests(&candidates, now);
        self.area_search_ui.table.pending = rows.iter().filter(|(r, _)| !r.loaded).count();
        // Name services are requested once per distinct identity per snapshot,
        // including filtered-out rows, so late replies can produce a match.
        let mut names = HashMap::new();
        let mut group_names = HashMap::new();
        for (r, _) in &rows {
            for id in [r.owner_id, r.creator_id, r.last_owner_id]
                .into_iter()
                .filter(|id| !id.is_nil() && *id != r.group_id)
            {
                names.entry(id).or_insert_with(|| {
                    self.world.social.want_name(id);
                    self.world.social.name_of(&id)
                });
            }
            if !r.group_id.is_nil() {
                group_names.entry(r.group_id).or_insert_with(|| {
                    self.world
                        .land
                        .group_name(&r.group_id, &self.world.groups.groups)
                        .unwrap_or_default()
                });
            }
        }
        let rows = rows
            .into_par_iter()
            .map(|(mut r, _)| {
                r.owner = if r.owner_id == r.group_id && !r.group_id.is_nil() {
                    group_names.get(&r.group_id).cloned().unwrap_or_default()
                } else {
                    names.get(&r.owner_id).cloned().unwrap_or_default()
                };
                r.creator = names.get(&r.creator_id).cloned().unwrap_or_default();
                r.last_owner = names.get(&r.last_owner_id).cloned().unwrap_or_default();
                r.group = group_names.get(&r.group_id).cloned().unwrap_or_default();
                r
            })
            .collect();
        self.area_search_ui.table.replace(rows);
        let cost_batches = if self.area_search_ui.needs_costs() {
            self.area_search_ui.table.prepare();
            let visible: HashSet<_> = self
                .area_search_ui
                .table
                .visible
                .iter()
                .map(|i| self.area_search_ui.table.rows[*i].id)
                .collect();
            let candidates: Vec<_> = candidates.into_iter().filter(|(id, _)| visible.contains(id)).collect();
            self.area_search_feed.cost_batches(&candidates, now)
        } else {
            HashMap::new()
        };
        let kept: HashSet<_> = self
            .build
            .highlighted(&self.world)
            .into_iter()
            .chain(self.inspect_prims())
            .filter_map(|(i, _)| self.world.objects.get(i).map(|o| o.key))
            .collect();
        for (handle, ids) in cost_batches {
            let body = aurora_llsd::llsd_map! { "object_ids" => aurora_llsd::Llsd::Array(ids.iter().copied().map(aurora_llsd::Llsd::from).collect()) };
            let tag = self.build.cap(
                handle,
                "GetObjectCost",
                false,
                body,
                crate::build::edits::CapPurpose::AreaSearchCost,
            );
            self.area_search_feed.cost_pending.insert(tag, (now, ids));
        }
        for command in property_commands(packets, &kept) {
            self.send(command);
        }
    }
}

fn property_commands(packets: HashMap<aurora_net::RegionHandle, Vec<u32>>, kept: &HashSet<ObjKey>) -> Vec<NetCommand> {
    let mut out = Vec::new();
    for (handle, local_ids) in packets {
        let deselect: Vec<_> = local_ids
            .iter()
            .copied()
            .filter(|id| {
                !kept.contains(&ObjKey {
                    region: handle,
                    local_id: *id,
                })
            })
            .collect();
        out.push(NetCommand::Build(BuildCmd::Select { handle, local_ids }));
        if !deselect.is_empty() {
            out.push(NetCommand::Build(BuildCmd::Deselect {
                handle,
                local_ids: deselect,
            }));
        }
    }
    out
}

/// Area reads have their own live cache. Filling Build's small cache with
/// thousands of unrelated results would evict its selected prims' properties.
fn for_build(
    props: Vec<ObjectProps>,
    area_reads: &HashSet<Uuid>,
    existing: &HashMap<Uuid, ObjectProps>,
    used: &HashSet<Uuid>,
) -> Vec<ObjectProps> {
    props
        .into_iter()
        .filter(|p| !p.full || !area_reads.contains(&p.object_id) || existing.contains_key(&p.object_id) || used.contains(&p.object_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn area_search_cost_requests_are_bounded_and_late_invalid_or_partial_replies_stay_unknown() {
        let now = Instant::now();
        let mut feed = Feed {
            seeded: true,
            ..Default::default()
        };
        let candidates: Vec<_> = (1..=400)
            .map(|id| {
                (
                    Uuid::from_u128(id),
                    ObjKey {
                        region: 1,
                        local_id: id as u32,
                    },
                )
            })
            .collect();
        for tag in 1..=3 {
            let batch = feed.cost_batches(&candidates, now);
            assert_eq!(batch[&1].len(), 64);
            feed.cost_pending.insert(tag, (now, batch[&1].clone()));
        }
        assert!(feed.cost_batches(&candidates, now).is_empty());
        feed.reset();
        assert!(
            feed.cost_batches(&candidates, now).is_empty(),
            "close/reopen cannot bypass the bound"
        );
        feed.seeded = true;
        let id = feed.cost_pending[&1].1[0];
        let missing = feed.cost_pending[&1].1[1];
        let mut reply = aurora_llsd::Llsd::new_map();
        reply.insert(id.to_string(), aurora_llsd::llsd_map! { "linked_set_resource_cost" => 2.5 });
        reply.insert(missing.to_string(), aurora_llsd::llsd_map! { "linked_set_resource_cost" => -1.0 });
        feed.on_cost_reply(1, &Ok(reply));
        assert_eq!(feed.costs[&id], 2.5);
        assert!(!feed.costs.contains_key(&missing));
        let batches = feed.cost_batches(&candidates, now);
        assert!(!batches[&1].contains(&missing), "partial replies wait before retrying");
        feed.on_cost_reply(2, &Err("offline".into()));
        feed.on_cost_reply(3, &Err("offline".into()));
        let later = feed.cost_batches(&candidates[..64], now + TIMEOUT);
        assert_eq!(later[&1].len(), 63, "known values are reused, missing values retry");
    }
    #[test]
    fn area_search_does_not_fill_the_build_cache_or_hide_selected_or_unrelated_replies() {
        let mut props: Vec<_> = (1..=6000)
            .map(|id| ObjectProps {
                object_id: Uuid::from_u128(id),
                full: true,
                ..Default::default()
            })
            .collect();
        let mut feed = Feed::default();
        feed.on_properties(&props);
        props[5998].full = false;
        let area_reads = props.iter().take(5999).map(|p| p.object_id).collect();
        let existing = HashMap::from([(props[0].object_id, props[0].clone())]);
        let used = HashSet::from([props[1].object_id]);
        let build_props = for_build(props, &area_reads, &existing, &used);
        assert_eq!(
            build_props.iter().map(|p| p.object_id.as_u128()).collect::<Vec<_>>(),
            [1, 2, 5999, 6000]
        );
        assert_eq!(
            feed.props.len(),
            6000,
            "Area Search retains all full details independently of Build's 4096-entry cache"
        );
    }

    #[test]
    fn area_search_batches_preserve_selection_and_bound_requests_across_scope_changes() {
        let now = Instant::now();
        let candidates: Vec<_> = (1..=2000)
            .map(|n| (Uuid::from_u128(n as u128), ObjKey { region: 1, local_id: n }))
            .collect();
        let mut feed = Feed::default();
        let kept = HashSet::from([candidates[0].1, candidates[3].1]);
        for _ in 0..3 {
            let packets = feed.requests(&candidates, now);
            assert_eq!(packets[&1].len(), PACKET);
            let selected = packets[&1].clone();
            let expected: Vec<_> = selected
                .iter()
                .copied()
                .filter(|id| !kept.contains(&ObjKey { region: 1, local_id: *id }))
                .collect();
            let commands = property_commands(packets, &kept);
            assert!(matches!(&commands[0], NetCommand::Build(BuildCmd::Select { local_ids, .. }) if local_ids == &selected));
            assert!(matches!(&commands[1], NetCommand::Build(BuildCmd::Deselect { local_ids, .. }) if local_ids == &expected));
        }
        assert_eq!(feed.requests.len(), IN_FLIGHT);
        assert!(
            feed.requests(&candidates[1500..], now).is_empty(),
            "changing scope cannot bypass outstanding limits"
        );
        feed.on_properties(&[ObjectProps {
            object_id: candidates[0].0,
            full: true,
            ..Default::default()
        }]);
        assert_eq!(feed.requests(&candidates, now)[&1].len(), 1);
        let packets = feed.requests(&candidates, now + TIMEOUT);
        assert_eq!(packets[&1].len(), PACKET, "lost replies can retry");
        assert!(!packets[&1].contains(&1), "completed metadata is retained");
    }

    #[test]
    fn area_search_waits_for_full_metadata_and_counts_each_region_independently() {
        let now = Instant::now();
        let mut feed = Feed::default();
        let candidates: Vec<_> = (1..=800)
            .flat_map(|n| {
                [1, 2].map(|region| {
                    (
                        Uuid::from_u128((region * 1000 + n) as u128),
                        ObjKey {
                            region,
                            local_id: n as u32,
                        },
                    )
                })
            })
            .collect();
        let packets = feed.requests(&candidates, now);
        assert_eq!(packets.len(), 2);
        let id = candidates[0].0;
        feed.on_properties(&[ObjectProps {
            object_id: id,
            full: false,
            ..Default::default()
        }]);
        assert!(feed.requests.contains_key(&id));
        assert!(!feed.props.contains_key(&id));
        feed.on_properties(&[ObjectProps {
            object_id: id,
            full: true,
            ..Default::default()
        }]);
        assert!(feed.expects(&id), "reliable duplicates retain their Area Search provenance");
        feed.reset();
        assert!(feed.props.is_empty());
        assert!(feed.expects(&id), "closing or crossing must still recognize late replies");
        let pending = candidates[1].0;
        feed.on_closed_properties(&[ObjectProps {
            object_id: pending,
            full: true,
            ..Default::default()
        }]);
        assert!(!feed.requests.contains_key(&pending));
        assert!(feed.props.is_empty(), "closed windows do not cache replies");
        let commands = property_commands(HashMap::from([(1, vec![7])]), &HashSet::from([ObjKey { region: 1, local_id: 7 }]));
        assert_eq!(commands.len(), 1, "a selected prim must stay selected on the simulator");
    }

    fn update(id: u32, parent: u32, pos: Vec3) -> aurora_net::objects::ObjectUpdate {
        let mut update = crate::demo::action_avatar(false);
        update.local_id = id;
        update.full_id = Uuid::from_u128(id as u128);
        update.parent_id = parent;
        update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        update.position = pos;
        update.rotation = Quat::IDENTITY;
        update
    }

    #[test]
    fn area_search_distinguishes_attachment_roots_and_children_and_rejects_invalid_ancestry() {
        let mut store = ObjectStore::default();
        let mut root = update(1, 0, Vec3::new(10.0, 20.0, 0.0));
        root.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        store.upsert(1, root);
        let child = Object::from_update(1, update(2, 1, Vec3::X * 3.0));
        assert!(placement(&store, &child).unwrap().position.distance(Vec3::new(10.0, 23.0, 0.0)) < 0.0001);
        assert!(placement(&store, &Object::from_update(1, update(3, 999, Vec3::ZERO))).is_none());
        let mut avatar = update(10, 0, Vec3::ZERO);
        avatar.pcode = 47;
        store.upsert(1, avatar);
        let root = placement(&store, &Object::from_update(1, update(11, 10, Vec3::ZERO))).unwrap();
        assert!(root.attachment && !root.child);
        store.upsert(1, update(12, 10, Vec3::ZERO));
        let child = placement(&store, &Object::from_update(1, update(13, 12, Vec3::ZERO))).unwrap();
        assert!(child.attachment && child.child);
        store.upsert(1, update(14, 15, Vec3::ZERO));
        store.upsert(1, update(15, 14, Vec3::ZERO));
        assert!(placement(&store, &Object::from_update(1, update(16, 14, Vec3::ZERO))).is_none());
        let local = Object::from_update(crate::world::objects::PREVIEW_REGION, update(17, 0, Vec3::ZERO));
        assert!(placement(&store, &local).is_none());
    }
}
