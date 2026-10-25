//! Bulk editing of agent inventory using the LLFloaterBulkPermission controls.
//! Item edits follow LLSidepanelItemInfo::onCommitPermissions
//! (indra/newview/llsidepaneliteminfo.cpp); folders include real descendants.

use super::*;
use crate::build::ui::contents_permissions::{BulkPerms, controls};
use aurora_llsd::llsd_map;
use aurora_net::inventory::InvItem;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct State {
    options: BulkPerms,
    original: Option<BulkPerms>,
    roots: Vec<Uuid>,
    open: bool,
    job: Option<Job>,
    status: String,
    submitted: bool,
}

struct Job {
    options: BulkPerms,
    todo: VecDeque<Uuid>,
    seen: HashSet<Uuid>,
    items: HashSet<Uuid>,
    started: Instant,
    plan: Option<VecDeque<Uuid>>,
    change: Mutation,
}

pub(super) fn eligible(inv: &Inventory, id: Uuid, prefs: &InventoryPreferences, agent: Uuid) -> bool {
    rules::mutable(inv, id, &prefs.protected)
        && !rules::in_type(inv, id, 14)
        && inv
            .items
            .get(&id)
            .is_none_or(|item| item.owner == agent && BulkPerms::default().includes_type(item.asset_type))
}

impl State {
    pub(super) fn active(&self) -> bool {
        self.open || self.job.is_some() || self.submitted
    }

    pub(super) fn open(&mut self, roots: Vec<Uuid>) {
        if self.job.is_some() || self.submitted {
            self.open = true;
            return;
        }
        self.roots = roots;
        self.original = Some(self.options.clone());
        self.status.clear();
        self.open = true;
    }

    fn start(&mut self) {
        self.job = Some(Job {
            options: self.options.clone(),
            todo: self.roots.iter().copied().collect(),
            seen: HashSet::new(),
            items: HashSet::new(),
            started: Instant::now(),
            plan: None,
            change: Mutation::default(),
        });
        self.status = aurora_i18n::tr!("inventory-bulk-reading").into();
    }
}

/// Preserve unrelated permission bits, never exceed the base rights and keep
/// no-copy transferable. Sharing requires both Copy and Transfer, just as in
/// the individual properties panel. Modify on the asset is not required.
fn permissions(item: &InvItem, options: &BulkPerms) -> (u32, u32, u32) {
    let sharing = rules::COPY | rules::TRANSFER;
    let group_bits = rules::COPY | rules::MODIFY | rules::MOVE;
    let (group, everyone) = if item.owner_mask & sharing == sharing {
        (
            (item.group_mask & !group_bits) | if options.share { group_bits & item.base_mask } else { 0 },
            (item.everyone_mask & !rules::COPY) | if options.everyone_copy { rules::COPY & item.base_mask } else { 0 },
        )
    } else {
        (item.group_mask, item.everyone_mask)
    };
    let mut next = 0;
    if options.next_modify {
        next |= rules::MODIFY;
    }
    if options.next_copy || item.asset_type == 56 {
        next |= rules::COPY;
    }
    if options.next_transfer || !options.next_copy {
        next |= rules::TRANSFER;
    }
    next &= item.base_mask;
    if next & (rules::COPY | rules::TRANSFER) == 0 {
        next |= item.base_mask & (rules::COPY | rules::TRANSFER);
    }
    next |= item.next_owner_mask & !(rules::COPY | rules::MODIFY | rules::TRANSFER);
    (group, everyone, next)
}

impl Job {
    /// Bound traversal work per frame and use the normal lazy-folder requests.
    /// No PATCH leaves until all selected folders are completely loaded.
    fn advance(&mut self, inv: &mut Inventory, prefs: &InventoryPreferences, agent: Uuid) -> Result<Option<Mutation>, &'static str> {
        if self.started.elapsed() > Duration::from_secs(120) {
            return Err(aurora_i18n::tr!("inventory-bulk-load-failed"));
        }
        if self.plan.is_none() {
            for _ in 0..256 {
                let Some(id) = self.todo.pop_front() else {
                    self.plan = Some(self.items.drain().collect());
                    break;
                };
                if self.seen.contains(&id) || !eligible(inv, id, prefs, agent) {
                    continue;
                }
                if let Some(folder) = inv.folders.get(&id) {
                    match folder.state {
                        FetchState::Unknown | FetchState::Fetching => {
                            self.todo.push_back(id);
                            if inv.queue.len() < 16 {
                                inv.request(id);
                            }
                            continue;
                        }
                        FetchState::Failed => return Err(aurora_i18n::tr!("inventory-bulk-load-failed")),
                        FetchState::Fetched => self.todo.extend(folder.children.iter().chain(&folder.items).copied()),
                    }
                } else if inv.items.contains_key(&id) {
                    self.items.insert(id);
                }
                self.seen.insert(id);
            }
        }
        if let Some(plan) = self.plan.as_mut() {
            for _ in 0..256 {
                let Some(id) = plan.pop_front() else {
                    self.change.refresh.sort_unstable();
                    self.change.refresh.dedup();
                    return Ok(Some(std::mem::take(&mut self.change)));
                };
                if !eligible(inv, id, prefs, agent) {
                    continue;
                }
                let Some(item) = inv.items.get(&id) else { continue };
                if !self.options.includes_type(item.asset_type) {
                    continue;
                }
                let (group, everyone, next) = permissions(item, &self.options);
                if (group, everyone, next) == (item.group_mask, item.everyone_mask, item.next_owner_mask) {
                    continue;
                }
                let masks = llsd_map! {
                    "group_mask" => group as i32, "everyone_mask" => everyone as i32, "next_owner_mask" => next as i32
                };
                let body = if item.asset_type == 6 {
                    // LLSidepanelItemInfo::updatePermissions: stored objects
                    // must carry these overrides so rez applies the new masks.
                    let mut flags = item.flags;
                    if next != item.next_owner_mask {
                        flags |= 0x100;
                    }
                    if group != item.group_mask {
                        flags |= 0x040000;
                    }
                    if everyone != item.everyone_mask {
                        flags |= 0x080000;
                    }
                    llsd_map! { "permissions" => masks, "flags" => flags as i32 }
                } else {
                    llsd_map! { "permissions" => masks }
                };
                let update = rules::patch(inv, id, body).map_err(|_| aurora_i18n::tr!("inventory-bulk-load-failed"))?;
                self.change.operations.extend(update.operations);
                self.change.refresh.extend(update.refresh);
            }
        }
        Ok(None)
    }
}

/// Open capture menus after the startup focus/size changes have settled.
pub fn demo_menu(frame: u64, st: &mut InventoryUi) {
    if frame == 650
        && let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY")
    {
        st.demo_menu = match mode.as_str() {
            "bulk-multi" => Some(Uuid::from_u128(8100)),
            "bulk-folder" => Some(Uuid::from_u128(8000)),
            _ => st.demo_menu,
        };
    }
}

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    agent: Uuid,
    actions: &mut Vec<InvAction>,
) {
    let progress = st.pending.map(|_| st.pending_status(Instant::now()));
    let state = &mut st.bulk_permissions;
    if state.job.is_some() && st.pending.is_none() && !actions.iter().any(|action| matches!(action, InvAction::Edit(_))) {
        let result = state.job.as_mut().and_then(|job| job.advance(inv, prefs, agent).transpose());
        if let Some(result) = result {
            state.job = None;
            match result {
                Ok(change) if !change.operations.is_empty() => {
                    state.status = aurora_i18n::tr!("content-perms-starting").into();
                    state.submitted = true;
                    actions.push(InvAction::Edit(change));
                }
                Ok(_) => state.status = aurora_i18n::tr!("inventory-bulk-unchanged").into(),
                Err(error) => {
                    state.status = error.into();
                    st.message = error.into();
                }
            }
        }
    }
    if !state.open {
        return;
    }
    let mut open = true;
    let mut response = (false, false, false);
    let busy =
        state.job.is_some() || state.submitted || st.pending.is_some() || actions.iter().any(|action| matches!(action, InvAction::Edit(_)));
    let status = progress.as_deref().unwrap_or(&state.status);
    crate::ui::widgets::Floater::new(
        &format!("inventory_bulk_permissions_{}", st.ui_id),
        aurora_i18n::tr!("inventory-bulk-permissions"),
        ctx.content_rect().center() - Vec2::new(180.0, 177.0),
        Vec2::new(360.0, 354.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        response = controls(ui, p, &mut state.options, status, busy, aurora_i18n::tr!("inventory-bulk-adjust"));
    });
    let (apply, accept, cancel) = response;
    if apply || accept {
        state.start();
    }
    if accept {
        state.original = None;
        open = false;
    }
    if cancel || !open && !accept {
        if let Some(original) = state.original.take() {
            state.options = original;
        }
        open = false;
    }
    state.open = open;
}

impl InventoryUi {
    /// The ordinary inventory result completes the originating bulk window too.
    pub(crate) fn finish_bulk_permissions(&mut self, error: Option<&str>) {
        let state = &mut self.bulk_permissions;
        if state.submitted {
            state.submitted = false;
            state.status = error.unwrap_or(aurora_i18n::tr!("inventory-operation-complete")).into();
        }
        for window in &mut self.windows {
            window.state.finish_bulk_permissions(error);
        }
    }
}

#[cfg(test)]
#[path = "bulk_permissions_tests.rs"]
mod tests;
