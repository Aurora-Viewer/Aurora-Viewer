//! Next-owner edits on selected real inventory records. Each request uses AIS and
//! reads its parents back. Object assets and object contents are never requested.
use super::{FetchState, InvAction, Inventory, InventoryUi, Palette, organized_model as model, rules};
use aurora_i18n::tr;
use aurora_llsd::llsd_map;
use aurora_net::inventory::{
    InvItem,
    operations::{Mutation, Reply},
};
use egui::Vec2;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SLAM_NEXT_OWNER: u32 = 0x100;
const BATCH: usize = 32;

pub(super) fn selectable(inv: &Inventory, id: Uuid, agent: Uuid) -> bool {
    if id.is_nil() || agent.is_nil() || inv.owner != agent {
        return false;
    }
    if let Some(item) = inv.items.get(&id) {
        !item.group_owned
            && item.owner == agent
            && rules::next_owner_editable(item)
            && item.asset_type >= 0
            && item.inv_type >= 0
            && !matches!(item.asset_type, 8 | 9 | 24 | 25)
            && !matches!(item.inv_type, 8 | 9)
            && model::personal_contents(inv, item.parent)
    } else {
        id != inv.root && inv.folders.contains_key(&id) && model::personal_contents(inv, id)
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Choice {
    #[default]
    Keep,
    Allow,
    Deny,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct Options {
    modify: Choice,
    copy: Choice,
    transfer: Choice,
}

impl Options {
    // Apply all requested choices or skip the item. Preserve unrelated bits and
    // never silently grant an alternative right to satisfy the fair-use rule.
    fn mask(self, inv: &Inventory, item: &InvItem) -> Option<u32> {
        let mut next = item.next_owner_mask;
        for (bit, choice) in [
            (rules::MODIFY, self.modify),
            (rules::COPY, self.copy),
            (rules::TRANSFER, self.transfer),
        ] {
            match choice {
                Choice::Keep => {}
                Choice::Allow if item.base_mask & rules::held(inv, item) & bit == 0 => return None,
                Choice::Allow => next |= bit,
                Choice::Deny => next &= !bit,
            }
        }
        if next & (rules::TRANSFER | rules::COPY) == 0 || item.asset_type == 56 && next & rules::COPY == 0 {
            return None;
        }
        Some(next)
    }
}

#[derive(Default)]
struct Counts {
    skipped: usize,
    unchanged: usize,
    unread: usize,
    verified: usize,
    unconfirmed: usize,
}

struct Review {
    roots: HashSet<Uuid>,
    queue: VecDeque<Uuid>,
    visited: HashSet<Uuid>,
    waiting: HashMap<Uuid, Instant>,
    ready: Vec<Uuid>,
    complete: bool,
    applying: bool,
    counts: Counts,
}

impl Review {
    fn new(roots: &[Uuid]) -> Self {
        Self {
            roots: roots.iter().copied().collect(),
            queue: roots.iter().copied().collect(),
            visited: Default::default(),
            waiting: Default::default(),
            ready: Vec::new(),
            complete: false,
            applying: false,
            counts: Default::default(),
        }
    }

    fn scan(&mut self, inv: &mut Inventory, agent: Uuid, options: Options) {
        let now = Instant::now();
        // Limit frame time and poll a waiting folder at most once per frame.
        for _ in 0..self.queue.len().min(256) {
            let Some(id) = self.queue.pop_front() else { break };
            if self.visited.contains(&id) {
                continue;
            }
            if !selectable(inv, id, agent) {
                if inv.folders.contains_key(&id) {
                    self.counts.unread += 1;
                } else {
                    self.counts.skipped += 1;
                }
            } else if let Some(folder) = inv.folders.get(&id) {
                match folder.state {
                    FetchState::Fetched => {
                        self.queue.extend(
                            folder
                                .children
                                .iter()
                                .copied()
                                .filter(|child| rules::parent(inv, *child) == Some(id)),
                        );
                        self.queue
                            .extend(folder.items.iter().copied().filter(|child| rules::parent(inv, *child) == Some(id)));
                    }
                    FetchState::Failed => self.counts.unread += 1,
                    FetchState::Unknown | FetchState::Fetching => {
                        // Time out an actual request, not a folder waiting behind a full queue.
                        if self
                            .waiting
                            .get(&id)
                            .is_some_and(|start| now.duration_since(*start) > Duration::from_secs(90))
                        {
                            self.counts.unread += 1;
                        } else {
                            if folder.state == FetchState::Fetching || inv.queue.len() < 16 {
                                self.waiting.entry(id).or_insert(now);
                                inv.request(id);
                            }
                            self.queue.push_back(id);
                            continue;
                        }
                    }
                }
            } else if let Some(item) = inv.items.get(&id) {
                match options.mask(inv, item) {
                    None => self.counts.skipped += 1,
                    Some(next) if next == item.next_owner_mask => self.counts.unchanged += 1,
                    Some(_) => self.ready.push(id),
                }
            }
            self.visited.insert(id);
            self.waiting.remove(&id);
        }
        if self.queue.is_empty() {
            // Batch siblings so each parent is fetched once per batch.
            self.ready.sort_unstable_by_key(|id| (rules::parent(inv, *id), *id));
            self.complete = true;
        }
    }

    fn batch(&mut self, inv: &Inventory, agent: Uuid, options: Options) -> (Mutation, HashMap<Uuid, (u32, bool)>) {
        let mut change = Mutation {
            bulk_next_owner: true,
            ..Default::default()
        };
        let mut expected = HashMap::new();
        let parent = self.ready.last().and_then(|id| rules::parent(inv, *id));
        for _ in 0..BATCH {
            let Some(&id) = self.ready.last() else { break };
            // Keep separate real folders independent on readback too. An
            // unreadable parent must not hide a sibling folder's confirmations.
            if rules::parent(inv, id) != parent {
                break;
            }
            self.ready.pop();
            if !selectable(inv, id, agent) || !rules::ancestors(inv, id).any(|id| self.roots.contains(&id)) {
                self.counts.skipped += 1;
                continue;
            }
            let Some(item) = inv.items.get(&id) else {
                self.counts.skipped += 1;
                continue;
            };
            let Some(next) = options.mask(inv, item) else {
                self.counts.skipped += 1;
                continue;
            };
            if next == item.next_owner_mask {
                self.counts.unchanged += 1;
                continue;
            }
            let permissions = llsd_map! { "next_owner_mask" => next as i32 };
            let object = item.asset_type == 6;
            let body = if object {
                // LLInventoryItemFlags::II_FLAGS_OBJECT_SLAM_PERM applies next owner on rez.
                llsd_map! { "permissions" => permissions, "flags" => aurora_llsd::Llsd::Binary((item.flags | SLAM_NEXT_OWNER).to_be_bytes().to_vec()) }
            } else {
                llsd_map! { "permissions" => permissions }
            };
            if let Ok(patch) = rules::patch(inv, id, body) {
                expected.insert(id, (next, object));
                change.operations.extend(patch.operations);
                change.refresh.extend(patch.refresh);
            } else {
                self.counts.skipped += 1;
            }
        }
        change.refresh.sort_unstable();
        change.refresh.dedup();
        (change, expected)
    }
}

#[derive(Default)]
pub(crate) struct State {
    account: Uuid,
    roots: Vec<Uuid>,
    options: Options,
    open: bool,
    review: Option<Review>,
    sent: HashMap<Uuid, (u32, bool)>,
    request: Option<Uuid>,
}

impl State {
    pub(super) fn running(&self) -> bool {
        !self.sent.is_empty() || self.review.as_ref().is_some_and(|r| r.applying && !r.ready.is_empty())
    }
    pub(super) fn open(&mut self, account: Uuid, mut roots: Vec<Uuid>) {
        roots.sort_unstable();
        roots.dedup();
        if self.account != account
            || !self.running() && (!self.open || self.roots != roots || self.review.as_ref().is_some_and(|r| r.applying))
        {
            *self = Self {
                account,
                roots,
                ..Default::default()
            };
        }
        self.open = true;
    }
    pub(crate) fn bind(&mut self, request: Uuid) {
        if !self.sent.is_empty() {
            self.request = Some(request);
        }
    }
    pub(crate) fn finish(&mut self, request: Uuid, reply: Option<&Reply>) {
        if self.request != Some(request) {
            return;
        }
        self.request = None;
        let Some(review) = &mut self.review else {
            self.sent.clear();
            return;
        };
        if let Some(reply) = reply {
            for item in reply.contents.iter().flat_map(|folder| &folder.items) {
                if self
                    .sent
                    .get(&item.id)
                    .is_some_and(|(mask, object)| item.next_owner_mask == *mask && (!object || item.flags & SLAM_NEXT_OWNER != 0))
                {
                    self.sent.remove(&item.id);
                    review.counts.verified += 1;
                }
            }
        }
        review.counts.unconfirmed += self.sent.len();
        self.sent.clear();
    }
}

fn picker(ui: &mut egui::Ui, label: &str, value: &mut Choice) {
    let choices = [
        (Choice::Keep, tr!("explorer-permissions-keep")),
        (Choice::Allow, tr!("explorer-permissions-allow")),
        (Choice::Deny, tr!("explorer-permissions-deny")),
    ];
    ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .selected_text(choices.iter().find(|(choice, _)| choice == value).unwrap().1)
        .show_ui(ui, |ui| {
            for (choice, text) in choices {
                ui.selectable_value(value, choice, text);
            }
        });
    ui.end_row();
}

pub(super) fn show(
    ctx: &egui::Context,
    palette: &Palette,
    inv: &mut Inventory,
    ui_state: &mut InventoryUi,
    agent: Uuid,
    actions: &mut Vec<InvAction>,
) {
    let state = &mut ui_state.next_owner;
    if state.account != inv.root {
        *state = Default::default();
    }
    if !state.open {
        return;
    }
    if state.request.is_none() && !state.sent.is_empty() {
        // A higher-priority command or edit guard declined it before dispatch.
        if let Some(review) = &mut state.review {
            review.counts.unconfirmed += state.sent.len();
        }
        state.sent.clear();
    }
    if let Some(review) = &mut state.review {
        if !review.complete {
            review.scan(inv, agent, state.options);
        } else if review.applying && state.sent.is_empty() && ui_state.pending.is_none() && actions.is_empty() {
            let (change, expected) = review.batch(inv, agent, state.options);
            if !expected.is_empty() {
                state.sent = expected;
                actions.push(InvAction::Edit(change));
            }
        }
    }
    let mut open = true;
    let mut back = false;
    super::widgets::Floater::new(
        "explorer_next_owner",
        tr!("explorer-permissions-title"),
        ctx.content_rect().center() - Vec2::new(210.0, 142.0),
        Vec2::new(420.0, 284.0),
    )
    .show(ctx, palette, &mut open, |ui| {
        let first = state.roots.first().map(|id| rules::name(inv, *id)).unwrap_or_default();
        let heading = tr!("explorer-permissions-selection", count = state.roots.len(), first = first);
        super::widgets::collapsing(ui, palette, "selection", heading, false, |ui| {
            egui::ScrollArea::vertical()
                .max_height(84.0)
                .show_rows(ui, 18.0, state.roots.len(), |ui, rows| {
                    for id in &state.roots[rows] {
                        let path = super::inventory_path(inv, *id);
                        ui.add(egui::Label::new(&path).truncate()).on_hover_text(path);
                    }
                });
        });
        if let Some(review) = &mut state.review {
            let counts = &review.counts;
            if !review.complete {
                ui.label(tr!("inventory-bulk-reading"));
            } else {
                ui.label(tr!(
                    "explorer-permissions-counts",
                    ready = review.ready.len(),
                    same = counts.unchanged,
                    skipped = counts.skipped,
                    folders = counts.unread
                ));
                if review.applying {
                    ui.label(tr!(
                        "explorer-permissions-results",
                        verified = counts.verified,
                        unconfirmed = counts.unconfirmed,
                        pending = state.sent.len()
                    ));
                    if !review.ready.is_empty() && ui.button(tr!("explorer-permissions-stop")).clicked() {
                        review.ready.clear();
                    }
                } else {
                    ui.horizontal(|ui| {
                        if ui.button(tr!("explorer-permissions-edit")).clicked() {
                            back = true;
                        }
                        if ui
                            .add_enabled(!review.ready.is_empty(), egui::Button::new(tr!("common-apply")))
                            .clicked()
                        {
                            review.applying = true;
                        }
                    });
                }
            }
        } else {
            ui.label(tr!("explorer-permissions-scope"));
            egui::Grid::new("next_owner_choices").show(ui, |ui| {
                picker(ui, tr!("explorer-permissions-modify"), &mut state.options.modify);
                picker(ui, tr!("explorer-permissions-copy"), &mut state.options.copy);
                picker(ui, tr!("explorer-permissions-transfer"), &mut state.options.transfer);
            });
            ui.label(tr!("explorer-permissions-rule"));
            let valid =
                state.options != Options::default() && !(state.options.copy == Choice::Deny && state.options.transfer == Choice::Deny);
            if ui
                .add_enabled(valid, egui::Button::new(tr!("explorer-permissions-review")))
                .clicked()
            {
                state.review = Some(Review::new(&state.roots));
            }
        }
    });
    if back {
        state.review = None;
    }
    if !open {
        // Closing during a run stops future batches. An outstanding request
        // can still complete and be verified through its own callback.
        if let Some(review) = &mut state.review {
            review.ready.clear();
        }
    }
    state.open = open;
}

#[cfg(test)]
#[path = "organized_permissions_tests.rs"]
mod tests;
