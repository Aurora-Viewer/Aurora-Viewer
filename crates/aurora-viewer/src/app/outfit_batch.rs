//! The outfits' batch portraits on the app's side (Fider #447): the window
//! is `ui::appearance::batch`, the photo the auto portrait's
//! (`app::outfit_portrait`, a session without its window).
//!
//! Started by a click, the batch takes the outfits ticked one by one: it
//! wears the outfit the way the gallery's Replace does (the same outfit
//! write, nothing more), waits until it has really loaded (the write done,
//! the Current Outfit linking it, every item worn, a new appearance, our
//! avatar shown in its new look, all of it for a moment), frames it, takes
//! the photo and saves it as the outfit's free picture, then moves to the
//! next. An outfit that does not load in time is skipped, or photographed
//! as it is once our avatar is drawn in it, and the window says how many
//! items were missing. Two outfit changes are always at least `MIN_GAP`
//! apart, as a person clicking would be. Pause holds the next outfit and
//! the next photo; Stop, the window's X or the end put the starting outfit
//! back. Outfits RLVa locks are not offered and skipped if locked since.
//!
//! The paid HD pictures, when ticked: the total for the outfits chosen
//! (count × the texture upload price) is confirmed once at Start against
//! the balance, like the auto portrait's single picture; each photo taken
//! then goes the auto portrait's way (`outfit_hd`), paid as it is sent, so
//! an outfit skipped costs nothing.

use super::App;
use crate::snapshot::portrait::{self, HdOffer, PortraitSettings};
use crate::ui::appearance::batch as window;
use crate::world::appearance as model;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The least time between two outfit changes.
const MIN_GAP: Duration = Duration::from_secs(5);
/// How long the outfit must look loaded before its photo (a texture or a
/// layer arriving late resets it).
const SETTLE: Duration = Duration::from_secs(2);
/// After this long, an outfit worn and drawn is photographed as it is, one
/// that is not is skipped.
const LOAD_LIMIT: Duration = Duration::from_secs(60);
/// The camera's framing (`outfit_portrait::REFRAME`) and a moment to hold
/// still before the photo.
const FRAME_WAIT: Duration = Duration::from_millis(2000);
/// A photo that never came back.
const SHOT_LIMIT: Duration = Duration::from_secs(20);
/// Putting the starting outfit back.
const RESTORE_LIMIT: Duration = Duration::from_secs(60);
/// The loading's signals are read this often (our avatar's prims and
/// textures are walked for them).
const CHECK_EVERY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq)]
enum State {
    Waiting,
    Loading {
        since: Instant,
        /// The outfit write is sent (or none was needed: `wore` false).
        sent: bool,
        wore: bool,
        /// Our appearance's COF version before the change.
        cof: i32,
        ready_since: Option<Instant>,
        worn: usize,
        total: usize,
    },
    Framing {
        until: Instant,
        missing: usize,
    },
    Shooting {
        since: Instant,
        missing: usize,
    },
    Saving {
        missing: usize,
    },
    Done {
        missing: usize,
    },
    NotDone,
    Skipped(window::Skip),
    Failed(String),
}

impl State {
    fn finished(&self) -> bool {
        matches!(self, State::Done { .. } | State::NotDone | State::Skipped(_) | State::Failed(_))
    }
}

struct Entry {
    outfit: Uuid,
    chosen: bool,
    had_picture: bool,
    locked: bool,
    empty: bool,
    state: State,
    /// The photo taken.
    picture: Option<egui::TextureHandle>,
}

/// Where putting the starting outfit back stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restore {
    /// The outfit to wear again.
    Outfit,
    /// The outfit sent.
    OutfitSent,
    /// The body parts worn at the start sent too: an outfit without one
    /// keeps those of the last outfit photographed (the gallery's Replace
    /// swaps body parts only for those it holds).
    PartsSent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Setup,
    Running { paused: bool },
    Restoring { since: Instant, step: Restore },
    Finished { restored: bool },
}

pub(super) struct Batch {
    entries: Vec<Entry>,
    phase: Phase,
    settings: PortraitSettings,
    /// The outfit worn at the start, put back at the end, and its body
    /// parts.
    start: Option<Uuid>,
    start_parts: Vec<Uuid>,
    last_wear: Option<Instant>,
    /// The loading's signals last read.
    checked: Option<Instant>,
    open: bool,
    /// Start waits for the HD pictures' total to be confirmed.
    hd_confirm: Option<i32>,
    /// The HD pictures confirmed at the start: their side and the price of
    /// each.
    hd: Option<(u32, i32)>,
}

/// What tells that an outfit has loaded.
#[derive(Debug, Clone, Copy, Default)]
struct Signals {
    /// The outfit write is still under way.
    pending: bool,
    /// The Current Outfit links this outfit.
    worn_base: bool,
    /// The outfit's body parts, clothes and objects worn, of `total`.
    worn: usize,
    total: usize,
    /// Our avatar is drawn in its new look (`Scene::own_look_complete`).
    look_complete: bool,
    /// The simulator sent the appearance of the new outfit (or we bake it
    /// ourselves, or no change was needed).
    appearance_new: bool,
}

impl Signals {
    fn loaded(&self) -> bool {
        !self.pending && self.worn_base && self.worn >= self.total && self.look_complete && self.appearance_new
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    Wait,
    /// Photographed, with this many items not worn.
    Shoot(usize),
    TooLong,
}

/// The outfit's loading, `waited` after its change, loaded for `settled`.
fn load_step(s: &Signals, waited: Duration, settled: Option<Duration>) -> Load {
    if s.loaded() {
        return if settled.is_some_and(|d| d >= SETTLE) {
            Load::Shoot(0)
        } else {
            Load::Wait
        };
    }
    if waited < LOAD_LIMIT {
        return Load::Wait;
    }
    if !s.pending && s.worn_base && s.look_complete {
        Load::Shoot(s.total.saturating_sub(s.worn))
    } else {
        Load::TooLong
    }
}

/// An outfit change may be sent.
fn wear_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= MIN_GAP)
}

/// The outfits in the showcase's order, the one worn first (no change for
/// it), each once.
fn order(outfits: &[Uuid], start: Option<Uuid>) -> Vec<Uuid> {
    let mut seen = std::collections::HashSet::new();
    start
        .filter(|id| outfits.contains(id))
        .into_iter()
        .chain(outfits.iter().copied())
        .filter(|id| seen.insert(*id))
        .collect()
}

/// The next outfit to photograph.
fn next(entries: &[Entry]) -> Option<usize> {
    entries.iter().position(|e| e.chosen && !e.state.finished())
}

/// The body parts the Current Outfit links, in order.
fn body_parts(inv: &crate::world::inventory::Inventory) -> Vec<Uuid> {
    let mut parts: Vec<Uuid> = model::cof(inv)
        .map(|cof| model::folder_links(inv, cof))
        .unwrap_or_default()
        .into_iter()
        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type == 13))
        .map(|l| l.target)
        .collect();
    parts.sort_unstable();
    parts.dedup();
    parts
}

/// The items an outfit puts on: its body parts, clothes and objects.
fn wearables(inv: &crate::world::inventory::Inventory, outfit: Uuid) -> Vec<Uuid> {
    model::usable_links(inv, outfit)
        .into_iter()
        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13)))
        .map(|l| l.target)
        .collect()
}

impl App {
    /// The batch portraits' window over these outfits (ours, in the
    /// showcase's order); the one running is brought back.
    pub(super) fn open_outfit_batch(&mut self, outfits: Vec<Uuid>) {
        if let Some(b) = &mut self.outfit_batch
            && matches!(b.phase, Phase::Running { .. } | Phase::Restoring { .. })
        {
            b.open = true;
            crate::ui::widgets::open_floater(&self.egui_ctx, window::WINDOW, &mut true);
            return;
        }
        let inv = &self.world.inventory;
        let locks = self.world.rlva_wear_locks();
        let start = model::base(inv);
        let entries: Vec<Entry> = order(&outfits, start)
            .into_iter()
            .filter(|id| inv.folders.contains_key(id))
            .map(|outfit| {
                let had_picture = !model::outfit_image(inv, outfit).is_nil();
                let locked = !locks.folder_allows(outfit, true);
                let empty = wearables(inv, outfit).is_empty() && model::complete(inv, outfit);
                Entry {
                    outfit,
                    chosen: !had_picture && !locked && !empty,
                    had_picture,
                    locked,
                    empty,
                    state: State::Waiting,
                    picture: None,
                }
            })
            .collect();
        log::info!(
            "outfit batch: opened over {} outfits, {} without a picture",
            entries.len(),
            entries.iter().filter(|e| e.chosen).count()
        );
        self.outfit_batch = Some(Batch {
            entries,
            phase: Phase::Setup,
            settings: PortraitSettings::opening(&self.settings.outfit_portrait),
            start,
            start_parts: Vec::new(),
            last_wear: None,
            checked: None,
            open: true,
            hd_confirm: None,
            hd: None,
        });
        crate::ui::widgets::open_floater(&self.egui_ctx, window::WINDOW, &mut true);
    }

    /// Why Start is not offered now.
    fn outfit_batch_blocked(&self) -> Option<window::Blocked> {
        let inv = &self.world.inventory;
        if model::base(inv).is_none() {
            Some(window::Blocked::NoStart)
        } else if model::dirty(inv) {
            Some(window::Blocked::Unsaved)
        } else {
            None
        }
    }

    /// The HD pictures for the outfits ticked: (the side, the price of
    /// each, the total's offer against the balance).
    fn outfit_batch_hd_offer(&self) -> Option<(u32, Option<i32>, HdOffer)> {
        let b = self.outfit_batch.as_ref()?;
        let side = b.settings.hd_side.side();
        let each = self.outfit_hd_price(side);
        let count = b.entries.iter().filter(|e| e.chosen).count();
        Some((side, each, portrait::hd_offer(portrait::hd_total(each, count), self.world.balance)))
    }

    /// Start: with the HD pictures ticked and payable, their total is
    /// confirmed first (`confirmed`: the total agreed to); a total that
    /// cannot be paid leaves them out, as their greyed box says.
    fn outfit_batch_start_paid(&mut self, confirmed: Option<i32>) {
        if self.outfit_batch_blocked().is_some() {
            return;
        }
        let hd = match self.outfit_batch_hd_offer() {
            Some((side, Some(each), HdOffer::Ready(total))) if self.outfit_batch.as_ref().is_some_and(|b| b.settings.hd) => {
                Some((side, each, total))
            }
            _ => None,
        };
        let Some(b) = &mut self.outfit_batch else {
            return;
        };
        if let Some((_, _, total)) = hd.filter(|(_, _, total)| *total > 0 && confirmed != Some(*total)) {
            b.hd_confirm = Some(total);
            return;
        }
        b.hd_confirm = None;
        b.hd = hd.map(|(side, each, _)| (side, each));
        if let Some((side, each, total)) = hd {
            log::info!("outfit batch: HD pictures {side} x {side} at L$ {each} each, L$ {total} confirmed");
        }
        self.inventory_images.hd.batch = Default::default();
        self.outfit_batch_start();
    }

    fn outfit_batch_start(&mut self) {
        if self.outfit_batch_blocked().is_some() {
            return;
        }
        let start = model::base(&self.world.inventory);
        let parts = body_parts(&self.world.inventory);
        // the auto portrait's window or the square photo mode give way
        if self.outfit_portrait.is_some() {
            self.close_outfit_portrait();
        }
        let Some(b) = &mut self.outfit_batch else {
            return;
        };
        b.start = start;
        b.start_parts = parts;
        b.phase = Phase::Running { paused: false };
        log::info!(
            "outfit batch: started on {} outfits, {start:?} put back at the end",
            b.entries.iter().filter(|e| e.chosen).count()
        );
    }

    /// The end, or Stop: the session over, what is left not done, the
    /// starting outfit back on.
    fn outfit_batch_stop(&mut self) {
        self.close_outfit_portrait_batch();
        let Some(b) = &mut self.outfit_batch else {
            return;
        };
        for e in b.entries.iter_mut().filter(|e| e.chosen) {
            e.state = match std::mem::replace(&mut e.state, State::NotDone) {
                // (its picture is on its way)
                State::Saving { missing } => State::Done { missing },
                done if done.finished() => done,
                _ => State::NotDone,
            };
        }
        b.phase = Phase::Restoring {
            since: Instant::now(),
            step: Restore::Outfit,
        };
        log::info!("outfit batch: stopping, the starting outfit back");
    }

    /// Every frame, before the auto portrait's: the batch's next step.
    pub(super) fn outfit_batch_frame(&mut self) {
        let Some(b) = &self.outfit_batch else {
            return;
        };
        match b.phase {
            Phase::Setup | Phase::Finished { .. } => {}
            Phase::Running { paused } => self.outfit_batch_run(paused),
            Phase::Restoring { since, step } => self.outfit_batch_restore(since, step),
        }
    }

    fn outfit_batch_run(&mut self, paused: bool) {
        let now = Instant::now();
        let Some(b) = &self.outfit_batch else {
            return;
        };
        let Some(i) = next(&b.entries) else {
            log::info!("outfit batch: every outfit done");
            self.outfit_batch_stop();
            return;
        };
        let outfit = b.entries[i].outfit;
        let settings = b.settings;
        let state = b.entries[i].state.clone();
        let last_wear = b.last_wear;
        let check_due = b.checked.is_none_or(|t| now.saturating_duration_since(t) >= CHECK_EVERY);
        // the photo mode opened over the batch closes its session: the
        // batch waits for it
        if matches!(state, State::Framing { .. } | State::Shooting { .. }) && !self.outfit_portrait_batch_open() && self.panels.snapshot {
            self.outfit_batch_set_paused(true);
            return;
        }
        let new = match state {
            State::Waiting => {
                let inv = &self.world.inventory;
                if !inv
                    .folders
                    .get(&outfit)
                    .is_some_and(|f| !f.library && f.info.type_default == model::FT_OUTFIT)
                {
                    Some(State::Skipped(window::Skip::Gone))
                } else if !self.world.rlva_wear_locks().folder_allows(outfit, true) {
                    Some(State::Skipped(window::Skip::Locked))
                } else if model::complete(inv, outfit) && wearables(inv, outfit).is_empty() {
                    Some(State::Skipped(window::Skip::Empty))
                } else if paused {
                    None
                } else {
                    Some(State::Loading {
                        since: now,
                        sent: false,
                        wore: false,
                        cof: -1,
                        ready_since: None,
                        worn: 0,
                        total: 0,
                    })
                }
            }
            State::Loading { sent: false, since, .. } => self.outfit_batch_wear(outfit, since, paused, last_wear, now),
            State::Loading { .. } if !check_due => None,
            State::Loading {
                since,
                wore,
                cof,
                ready_since,
                ..
            } => {
                if let Some(b) = &mut self.outfit_batch {
                    b.checked = Some(now);
                }
                let inv = &self.world.inventory;
                let items = wearables(inv, outfit);
                let on = crate::ui::appearance::items::worn_items(&self.world);
                let worn_base = model::base(inv) == Some(outfit);
                let signals = Signals {
                    pending: self.appearance_ui.pending.is_some(),
                    worn_base,
                    worn: items.iter().filter(|id| on.contains(id)).count(),
                    total: items.len(),
                    look_complete: self.scene.own_look_complete(&self.world),
                    appearance_new: !wore
                        || self.demo
                        || self.world.bakes_locally()
                        || self.world.appearance_cof_version(&self.world.agent_id) > cof,
                };
                let ready_since = if signals.loaded() { ready_since.or(Some(now)) } else { None };
                let failed = !signals.pending && !worn_base && !self.appearance_ui.message.is_empty();
                match load_step(&signals, now.saturating_duration_since(since), ready_since.map(|t| now - t)) {
                    _ if failed => Some(State::Failed(self.appearance_ui.message.clone())),
                    Load::Wait => Some(State::Loading {
                        since,
                        sent: true,
                        wore,
                        cof,
                        ready_since,
                        worn: signals.worn,
                        total: signals.total,
                    }),
                    Load::Shoot(missing) => {
                        log::info!(
                            "outfit batch: {outfit} loaded in {:.1} s, {missing} items missing",
                            now.saturating_duration_since(since).as_secs_f32()
                        );
                        self.outfit_portrait_batch(outfit, settings);
                        Some(State::Framing {
                            until: now + FRAME_WAIT,
                            missing,
                        })
                    }
                    Load::TooLong => {
                        log::warn!("outfit batch: {outfit} still loading after {} s: skipped", LOAD_LIMIT.as_secs());
                        Some(State::Skipped(window::Skip::Slow(LOAD_LIMIT.as_secs())))
                    }
                }
            }
            State::Framing { until, missing } => {
                if !self.outfit_portrait_batch_open() {
                    // (closed by the photo mode, since closed again)
                    self.outfit_portrait_batch(outfit, settings);
                    Some(State::Framing {
                        until: now + FRAME_WAIT,
                        missing,
                    })
                } else if now >= until && !paused && !self.outfit_photo_busy() {
                    self.outfit_portrait_shoot();
                    Some(State::Shooting { since: now, missing })
                } else {
                    None
                }
            }
            State::Shooting { since, missing } => {
                if let Some(shot) = self.outfit_portrait_batch_shot() {
                    let texture = shot.texture.clone();
                    if let Some(b) = &mut self.outfit_batch {
                        b.entries[i].picture = Some(texture);
                    }
                    let hd = self.outfit_batch.as_ref().and_then(|b| b.hd);
                    let hd_pixels = hd.map(|_| shot.pixels.clone());
                    if self.outfit_photo_pixels(outfit, shot.size, shot.pixels) {
                        log::info!("outfit batch: photo of {outfit} saving");
                        if let (Some((side, each)), Some(pixels)) = (hd, hd_pixels) {
                            self.outfit_hd_start(outfit, shot.size, pixels, side, each, true);
                        }
                        Some(State::Saving { missing })
                    } else {
                        Some(State::Failed(self.appearance_ui.message.clone()))
                    }
                } else if !self.outfit_portrait_batch_open() {
                    Some(State::Framing {
                        until: now + FRAME_WAIT,
                        missing,
                    })
                } else if now.saturating_duration_since(since) >= SHOT_LIMIT {
                    Some(State::Failed(aurora_i18n::tr!("photo-draw-failed").into()))
                } else {
                    None
                }
            }
            State::Saving { missing } => (!self.outfit_photo_busy()).then(|| {
                if self.appearance_ui.message.is_empty() {
                    log::info!("outfit batch: picture of {outfit} saved");
                    State::Done { missing }
                } else {
                    State::Failed(self.appearance_ui.message.clone())
                }
            }),
            _ => None,
        };
        if let Some(new) = new
            && let Some(b) = &mut self.outfit_batch
        {
            b.entries[i].state = new;
        }
    }

    /// The outfit put on, once nothing else is under way and the last
    /// change is `MIN_GAP` behind; none when it is already worn.
    fn outfit_batch_wear(&mut self, outfit: Uuid, since: Instant, paused: bool, last: Option<Instant>, now: Instant) -> Option<State> {
        let inv = &self.world.inventory;
        let loading = |sent, wore, cof| State::Loading {
            since: now,
            sent,
            wore,
            cof,
            ready_since: None,
            worn: 0,
            total: 0,
        };
        if model::base(inv) == Some(outfit) && !model::dirty(inv) {
            return Some(loading(true, false, -1));
        }
        let ready = !paused && self.appearance_ui.pending.is_none() && wear_due(last, now) && model::complete(inv, outfit);
        if !ready {
            if now.saturating_duration_since(since) >= LOAD_LIMIT {
                return Some(State::Skipped(window::Skip::Slow(LOAD_LIMIT.as_secs())));
            }
            return None;
        }
        let cof = self.world.appearance_cof_version(&self.world.agent_id);
        log::info!("outfit batch: wearing {outfit}");
        self.appearance_ui.message.clear();
        self.apply_appearance_action(model::Action::Wear(outfit, false));
        if let Some(b) = &mut self.outfit_batch {
            b.last_wear = Some(now);
        }
        if self.appearance_ui.pending.is_some() || model::base(&self.world.inventory) == Some(outfit) {
            return Some(loading(true, true, cof));
        }
        // refused before anything was sent
        Some(if self.appearance_ui.message.is_empty() {
            State::Skipped(window::Skip::Refused)
        } else {
            State::Failed(self.appearance_ui.message.clone())
        })
    }

    fn outfit_batch_restore(&mut self, since: Instant, step: Restore) {
        let now = Instant::now();
        let inv = &self.world.inventory;
        let Some(b) = &mut self.outfit_batch else {
            return;
        };
        let pending = self.appearance_ui.pending.is_some();
        let back = b.start.is_some_and(|start| model::base(inv) == Some(start));
        let parts_back = b.start_parts.is_empty() || body_parts(inv) == b.start_parts;
        let due = !pending && wear_due(b.last_wear, now);
        let late = now.saturating_duration_since(since) >= RESTORE_LIMIT;
        let phase = match step {
            _ if b.start.is_none() => Some(Phase::Finished { restored: false }),
            _ if back && parts_back && !pending => Some(Phase::Finished { restored: true }),
            _ if late => Some(Phase::Finished { restored: false }),
            Restore::Outfit => due.then_some(Phase::Restoring {
                since,
                step: Restore::OutfitSent,
            }),
            Restore::OutfitSent if pending => None,
            Restore::OutfitSent if !back => Some(Phase::Finished { restored: false }),
            Restore::OutfitSent => due.then_some(Phase::Restoring {
                since,
                step: Restore::PartsSent,
            }),
            Restore::PartsSent if pending => None,
            // (a body part RLVa holds, or gone since)
            Restore::PartsSent => Some(Phase::Finished { restored: false }),
        };
        let Some(phase) = phase else {
            return;
        };
        let start = b.start;
        let parts = b.start_parts.clone();
        b.phase = phase;
        match phase {
            Phase::Restoring {
                step: Restore::OutfitSent, ..
            } => {
                if let Some(start) = start {
                    log::info!("outfit batch: putting {start} back");
                    b.last_wear = Some(now);
                    self.apply_appearance_action(model::Action::Wear(start, false));
                }
            }
            Phase::Restoring {
                step: Restore::PartsSent, ..
            } => {
                log::info!("outfit batch: putting the {} starting body parts back", parts.len());
                b.last_wear = Some(now);
                self.apply_appearance_action(model::Action::WearItems {
                    items: parts,
                    replace: true,
                    point: 0,
                });
            }
            Phase::Finished { restored } => log::info!("outfit batch: finished, starting outfit back: {restored}"),
            _ => {}
        }
    }

    fn outfit_batch_set_paused(&mut self, paused: bool) {
        if let Some(b) = &mut self.outfit_batch
            && let Phase::Running { paused: was } = &mut b.phase
            && *was != paused
        {
            *was = paused;
            log::info!("outfit batch: {}", if paused { "paused" } else { "resumed" });
        }
    }

    fn outfit_batch_action(&mut self, action: window::Action) {
        let Some(b) = &mut self.outfit_batch else {
            return;
        };
        match action {
            window::Action::Toggle(i) => {
                if b.phase == Phase::Setup
                    && let Some(e) = b.entries.get_mut(i)
                    && !e.locked
                    && !e.empty
                {
                    e.chosen = !e.chosen;
                }
            }
            window::Action::Start => self.outfit_batch_start_paid(None),
            window::Action::ConfirmHd => {
                let total = b.hd_confirm;
                self.outfit_batch_start_paid(total);
            }
            window::Action::CancelHd => b.hd_confirm = None,
            window::Action::Pause => self.outfit_batch_set_paused(true),
            window::Action::Resume => self.outfit_batch_set_paused(false),
            window::Action::Stop => {
                if matches!(b.phase, Phase::Running { .. }) {
                    self.outfit_batch_stop();
                }
            }
            window::Action::Close => match b.phase {
                Phase::Running { .. } => self.outfit_batch_stop(),
                Phase::Restoring { .. } => {}
                Phase::Setup | Phase::Finished { .. } => {
                    // the choices made before the start kept, as the auto
                    // portrait keeps them
                    if b.phase == Phase::Setup && self.settings.outfit_portrait != b.settings {
                        self.settings.outfit_portrait = b.settings;
                        self.settings.save();
                    }
                    self.outfit_batch = None;
                }
            },
        }
    }

    fn outfit_batch_view(&self) -> Option<window::View<'_>> {
        let b = self.outfit_batch.as_ref()?;
        let inv = &self.world.inventory;
        let setup = b.phase == Phase::Setup;
        let current = matches!(b.phase, Phase::Running { .. }).then(|| next(&b.entries)).flatten();
        let preview = self.outfit_portrait_batch_preview();
        let rows = b
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| setup || e.chosen)
            .map(|(i, e)| {
                let image = model::outfit_image(inv, e.outfit);
                let picture = if current == Some(i) && matches!(e.state, State::Framing { .. } | State::Shooting { .. }) {
                    preview
                } else {
                    e.picture.as_ref()
                };
                let state = match &e.state {
                    _ if setup && e.locked => window::RowState::Locked,
                    _ if setup && e.empty => window::RowState::Empty,
                    _ if setup => window::RowState::Choose {
                        chosen: e.chosen,
                        had_picture: e.had_picture,
                    },
                    State::Waiting => window::RowState::Waiting,
                    State::Loading { sent: false, .. } => window::RowState::Wearing,
                    State::Loading { worn, total, .. } if worn < total => window::RowState::Loading {
                        worn: *worn,
                        total: *total,
                    },
                    State::Loading { .. } => window::RowState::LoadingLook,
                    State::Framing { .. } => window::RowState::Framing,
                    State::Shooting { .. } => window::RowState::Shooting,
                    State::Saving { .. } => window::RowState::Saving,
                    State::Done { missing } => window::RowState::Done { missing: *missing },
                    State::NotDone => window::RowState::NotDone,
                    State::Skipped(skip) => window::RowState::Skipped(*skip),
                    State::Failed(reason) => window::RowState::Failed(reason),
                };
                window::Row {
                    name: inv.folders.get(&e.outfit).map_or("", |f| f.info.name.as_str()),
                    picture: picture.or_else(|| self.ui_images.get(&image).filter(|_| !image.is_nil())),
                    redo: !setup && e.had_picture,
                    current: current == Some(i),
                    state,
                }
            })
            .collect();
        let chosen = b.entries.iter().filter(|e| e.chosen).count();
        let finished = b.entries.iter().filter(|e| e.chosen && e.state.finished()).count();
        let phase = match b.phase {
            Phase::Setup => window::Phase::Setup(self.outfit_batch_blocked()),
            Phase::Running { paused } => window::Phase::Running {
                index: (finished + 1).min(chosen),
                count: chosen,
                done: finished,
                paused,
            },
            Phase::Restoring { .. } => window::Phase::Restoring,
            Phase::Finished { restored } => window::Phase::Finished {
                done: b.entries.iter().filter(|e| matches!(e.state, State::Done { .. })).count(),
                skipped: b
                    .entries
                    .iter()
                    .filter(|e| matches!(e.state, State::Skipped(_) | State::Failed(_)))
                    .count(),
                restored,
            },
        };
        let tally = &self.inventory_images.hd.batch;
        let hd = window::HdTotal {
            offer: self.outfit_batch_hd_offer().map(|(_, _, offer)| offer),
            each: self.outfit_batch_hd_offer().and_then(|(_, each, _)| each),
            count: chosen,
            balance: self.world.balance,
            confirm: b.hd_confirm,
            used: b.hd.is_some(),
            saved: tally.saved,
            paid: tally.paid,
            pending: self.inventory_images.hd.batch_pending(),
            failed: tally.failed,
            error: tally.error.as_deref(),
        };
        Some(window::View { phase, rows, hd })
    }

    /// The batch portraits' window.
    pub(super) fn outfit_batch_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(b) = &self.outfit_batch else {
            return;
        };
        let mut settings = b.settings;
        let mut open = b.open;
        let actions = match self.outfit_batch_view() {
            Some(view) => window::window(ctx, p, &mut settings, &view, &mut open),
            None => return,
        };
        if let Some(b) = &mut self.outfit_batch {
            b.settings = settings;
        }
        if !open {
            self.outfit_batch_action(window::Action::Close);
        }
        for action in actions {
            self.outfit_batch_action(action);
        }
    }
}

/// `AURORA_DEMO_APPEARANCE=batch|batch-run|batch-hd|batch-hd-poor`.
fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_APPEARANCE").ok().filter(|m| m.starts_with("batch")))
        .as_deref()
}

impl App {
    /// The demo's steps (`AURORA_DEMO_APPEARANCE`): over the showcase, at
    /// frame 600 the batch portraits' window over the demo's outfits
    /// (`batch`); `batch-run` saves the current outfit's changes at frame
    /// 640 and starts it at frame 700. `batch-hd`: the HD pictures ticked
    /// at 1024 with a fictional L$ 500 balance (the demo's price is L$ 10
    /// each), the changes saved at 640, Start left to the user;
    /// `batch-hd-poor`: a L$ 15 balance, short of the total.
    pub(super) fn outfit_batch_demo(&mut self) {
        if !self.demo {
            return;
        }
        let Some(mode) = demo_mode() else {
            return;
        };
        match self.frame_count {
            600 => {
                let inv = &self.world.inventory;
                let mut outfits: Vec<Uuid> = inv
                    .folders
                    .values()
                    .filter(|f| !f.library && f.info.type_default == model::FT_OUTFIT)
                    .map(|f| f.info.id)
                    .collect();
                outfits.sort_by_key(|id| inv.folders.get(id).map(|f| f.info.name.to_lowercase()));
                log::info!("demo: outfit batch ({mode})");
                self.open_outfit_batch(outfits);
                if mode.starts_with("batch-hd") {
                    if let Some(b) = &mut self.outfit_batch {
                        b.settings.hd = true;
                        b.settings.hd_side = portrait::HdSide::Side1024;
                    }
                    self.world.balance = Some(if mode == "batch-hd-poor" { 15 } else { 500 });
                }
            }
            // (the demo's current outfit has unsaved changes: saved first,
            // as Start asks)
            640 if (mode == "batch-run" || mode.starts_with("batch-hd")) && model::dirty(&self.world.inventory) => {
                self.apply_appearance_action(model::Action::Save(None));
            }
            700 if mode == "batch-run" => self.outfit_batch_start(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Signals {
        Signals {
            pending: false,
            worn_base: true,
            worn: 21,
            total: 21,
            look_complete: true,
            appearance_new: true,
        }
    }

    #[test]
    fn an_outfit_is_photographed_once_it_has_held_loaded() {
        let s = loaded();
        assert_eq!(load_step(&s, Duration::from_secs(3), None), Load::Wait, "just loaded");
        assert_eq!(load_step(&s, Duration::from_secs(3), Some(Duration::from_millis(500))), Load::Wait);
        assert_eq!(load_step(&s, Duration::from_secs(3), Some(SETTLE)), Load::Shoot(0));
    }

    #[test]
    fn every_signal_holds_the_photo_back() {
        let cases: [fn(&mut Signals); 5] = [
            |s| s.pending = true,
            |s| s.worn_base = false,
            |s| s.worn = 14,
            |s| s.look_complete = false,
            |s| s.appearance_new = false,
        ];
        for (i, change) in cases.iter().enumerate() {
            let mut s = loaded();
            change(&mut s);
            assert!(!s.loaded(), "case {i}");
            assert_eq!(load_step(&s, Duration::from_secs(10), Some(SETTLE)), Load::Wait, "case {i}");
        }
    }

    #[test]
    fn after_the_limit_a_drawn_outfit_is_taken_as_it_is_and_another_skipped() {
        let mut s = loaded();
        s.worn = 14;
        assert_eq!(load_step(&s, LOAD_LIMIT, None), Load::Shoot(7), "7 items never came");
        s.appearance_new = false;
        assert_eq!(load_step(&s, LOAD_LIMIT, None), Load::Shoot(7), "the look is drawn");
        s.look_complete = false;
        assert_eq!(load_step(&s, LOAD_LIMIT, None), Load::TooLong);
        let mut s = loaded();
        s.worn_base = false;
        assert_eq!(load_step(&s, LOAD_LIMIT, None), Load::TooLong, "never worn");
        let mut s = loaded();
        s.pending = true;
        assert_eq!(load_step(&s, LOAD_LIMIT, None), Load::TooLong, "the write never ended");
    }

    #[test]
    fn two_outfit_changes_are_at_least_the_gap_apart() {
        let now = Instant::now();
        assert!(wear_due(None, now));
        assert!(!wear_due(Some(now), now));
        assert!(!wear_due(Some(now), now + MIN_GAP - Duration::from_millis(1)));
        assert!(wear_due(Some(now), now + MIN_GAP));
        // a clock read before the last change never sends early
        assert!(!wear_due(Some(now + Duration::from_secs(1)), now));
    }

    #[test]
    fn the_worn_outfit_comes_first_and_each_outfit_once() {
        let [a, b, c] = [1, 2, 3].map(Uuid::from_u128);
        assert_eq!(order(&[a, b, c], Some(b)), vec![b, a, c]);
        assert_eq!(order(&[a, b, a, c], None), vec![a, b, c]);
        assert_eq!(order(&[a, c], Some(b)), vec![a, c], "the worn outfit not shown is left out");
    }

    #[test]
    fn the_next_outfit_is_the_first_chosen_not_finished() {
        let entry = |chosen, state| Entry {
            outfit: Uuid::nil(),
            chosen,
            had_picture: false,
            locked: false,
            empty: false,
            state,
            picture: None,
        };
        let entries = vec![
            entry(true, State::Done { missing: 0 }),
            entry(false, State::Waiting),
            entry(true, State::Skipped(window::Skip::Locked)),
            entry(true, State::Saving { missing: 0 }),
            entry(true, State::Waiting),
        ];
        assert_eq!(next(&entries), Some(3), "a picture on its way holds the next outfit");
        assert_eq!(next(&entries[..3]), None);
    }
}
