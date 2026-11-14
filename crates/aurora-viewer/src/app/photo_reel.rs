//! The photo reel of the session (`snapshot::reel`, `ui::photo_reel`):
//! every photo taken joins it, whatever its destination; its thumbnail and
//! the reel's own copy of its pixels are made on a worker. The photo being
//! sent is followed from its start to its outcome: one at a time, like the
//! photo window's (`photo_busy`). Sent again, a photo of the reel is read
//! back on a worker and held like a freeze frame: the destination's own
//! flow sends it (the inventory's price, the profile's caption, the
//! e-mail's recipient). Deleted, its files go to the Recycle Bin and its
//! Photo Album item to the inventory's Trash.

use super::App;
use crate::snapshot::reel::{self, Place, Reel};
use crate::snapshot::{inventory, save};
use crate::ui::photo_reel::{QuickView, ReelAction, ReelView};
use crate::ui::snapshot::Destination;
use aurora_render::Renderer;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

mod demo;

type Pixels = (u32, u32, Vec<u8>);

/// A worker's result.
enum Done {
    Thumb(u64, Pixels),
    Copied(u64, Result<PathBuf, String>),
    /// The quick view's picture of a photo.
    Full(u64, Result<Pixels, String>),
    /// A photo read back to be sent again, fitted to its destination.
    Held(u64, Destination, (u32, u32), Result<Pixels, String>),
    /// Its files sent to the Recycle Bin.
    Recycled(Result<(), String>),
}

pub(super) struct ReelState {
    pub reel: Reel,
    pub thumbs: HashMap<u64, egui::TextureHandle>,
    /// The quick view's pictures, at the screen's size: the photo shown and
    /// its neighbours, read ahead.
    pub full: HashMap<u64, egui::TextureHandle>,
    loading: HashSet<u64>,
    pub quick: QuickView,
    /// The session's copies, opened with the first one.
    folder: Option<reel::Folder>,
    tx: crossbeam_channel::Sender<Done>,
    rx: crossbeam_channel::Receiver<Done>,
    /// The photo a destination is sending.
    sending: Option<u64>,
    /// It is one of the reel sent again from the quick view, not a new
    /// photo: it stays in sight, and no card shows it when it arrives.
    again: bool,
    /// The last photo sent was sent again: its card is not shown.
    pub sent_again: bool,
    /// The held photo is this one of the reel, to be sent again.
    pub held: Option<u64>,
    /// A photo being read back to be held.
    holding: Option<u64>,
    /// A photo read back for the inventory, waiting for its price to be
    /// confirmed.
    pub pending: Option<(u64, Pixels)>,
    /// The photo the e-mail form of the quick view is for.
    pub mail: Option<u64>,
    /// The photo the prompt to make it our profile picture is for.
    pub apply: Option<u64>,
    /// The quick view was opened in photo mode (it closes with it), not
    /// from the toolbar's Photo roll without it.
    pub in_photo_mode: bool,
    /// The photo the last photo's card shows.
    pub card: Option<u64>,
    /// AURORA_DEMO_PHOTO_ROLL (`demo`).
    demo: Option<demo::Demo>,
}

impl Default for ReelState {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        ReelState {
            reel: Reel::default(),
            thumbs: HashMap::new(),
            full: HashMap::new(),
            loading: HashSet::new(),
            quick: QuickView::default(),
            folder: None,
            tx,
            rx,
            sending: None,
            again: false,
            sent_again: false,
            held: None,
            holding: None,
            pending: None,
            mail: None,
            apply: None,
            in_photo_mode: false,
            card: None,
            demo: None,
        }
    }
}

/// Delete the reel's copies, off the frame.
fn delete_files(files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for file in files {
            if let Err(e) = std::fs::remove_file(&file)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("photo reel: {} not deleted: {e}", file.display());
            }
        }
    });
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Run `work` on a worker of the reel, its result sent back.
fn spawn(tx: &crossbeam_channel::Sender<Done>, work: impl FnOnce() -> Done + Send + 'static) {
    let tx = tx.clone();
    let spawned = std::thread::Builder::new().name("aurora-photo-reel".into()).spawn(move || {
        let _ = tx.send(work());
    });
    if let Err(e) = spawned {
        log::warn!("photo reel: cannot start its worker: {e}");
    }
}

impl ReelState {
    /// What the reel shows: not a new photo still on its way (it joins
    /// when it is saved, uploaded, posted or sent, or when that fails).
    pub(super) fn view<'a>(&'a self, can_send: &'a [Destination], time: &'a dyn Fn(i64) -> String) -> ReelView<'a> {
        let photos = self.reel.photos();
        let on_its_way = self.sending.is_some() && !self.again && photos.first().map(|p| p.id) == self.sending;
        ReelView {
            photos: &photos[usize::from(on_its_way)..],
            thumbs: &self.thumbs,
            full: &self.full,
            can_send,
            time,
            arriving: false,
        }
    }

    /// Forget photos (pushed out, deleted, the session over): their
    /// pictures, and the reel's copies on a worker.
    fn forget(&mut self, photos: Vec<reel::Photo>) {
        let mut copies = Vec::new();
        for photo in photos {
            self.thumbs.remove(&photo.id);
            self.full.remove(&photo.id);
            if self.card == Some(photo.id) {
                self.card = None;
            }
            if self.held == Some(photo.id) {
                self.held = None;
            }
            if let Some(file) = photo.file.filter(|_| photo.own_copy) {
                copies.push(file);
            }
        }
        delete_files(copies);
    }

    /// A destination starts with a photo: the held reel photo sent again,
    /// or a new one joining the reel, its thumbnail and the reel's copy made
    /// from `rgba`, in the session's folder of the account `agent`.
    pub(super) fn start(&mut self, size: (u32, u32), rgba: &[u8], cutout: bool, agent: &uuid::Uuid) {
        if let Some(id) = self.held.filter(|id| self.reel.get(*id).is_some()) {
            self.sending = Some(id);
            self.again = true;
            return;
        }
        let (id, dropped) = self.reel.add(unix_now(), size, cutout, None, true, Vec::new());
        self.forget(dropped);
        self.sending = Some(id);
        self.again = false;
        if self.folder.is_none() {
            match reel::Folder::open(&reel::account_root(&crate::settings::cache_dir(), agent)) {
                Ok(folder) => self.folder = Some(folder),
                Err(e) => log::warn!("photo reel: no folder for the copies: {e}"),
            }
        }
        let copy = self.folder.as_ref().map(|f| f.path(id));
        let rgba = rgba.to_vec();
        let tx = self.tx.clone();
        spawn(&self.tx, move || {
            let _ = tx.send(Done::Thumb(id, save::thumbnail(size.0, size.1, &rgba)));
            let Some(path) = copy else {
                return Done::Copied(id, Err("no folder".into()));
            };
            let t0 = std::time::Instant::now();
            let result = reel::write_copy(&path, size, &rgba).map(|()| path);
            if result.is_ok() {
                log::info!(
                    "photo reel: copy of {} x {} written in {} ms",
                    size.0,
                    size.1,
                    t0.elapsed().as_millis()
                );
            }
            Done::Copied(id, result)
        });
    }

    /// The photo being sent arrived (`place`), or failed (None). A photo
    /// never written anywhere leaves the reel (a save to disk that failed:
    /// nothing to send again); one with its own copy stays, to be sent
    /// again. `thumb`: the save's thumbnail.
    pub(super) fn done(&mut self, place: Option<Place>, thumb: Option<egui::TextureHandle>) {
        let Some(id) = self.sending.take() else {
            return;
        };
        self.sent_again = std::mem::take(&mut self.again);
        if let Some(tex) = thumb {
            self.thumbs.entry(id).or_insert(tex);
        }
        match place {
            Some(place) => {
                self.reel.sent(id, place);
                if !self.sent_again {
                    self.card = Some(id);
                }
            }
            None => {
                if self.reel.get(id).is_some_and(|p| !p.own_copy && p.places.is_empty())
                    && let Some(photo) = self.reel.remove(id)
                {
                    self.forget(vec![photo]);
                }
            }
        }
    }

    /// The quick view reads its photo and the two beside it; the others'
    /// pictures are let go.
    fn read_ahead(&mut self, side: u32) {
        let shown = self.quick.open.filter(|_| !self.reel.is_empty());
        let Some(cursor) = shown.and_then(|id| self.reel.index_of(id)) else {
            self.full.clear();
            return;
        };
        let photos = self.reel.photos();
        let wanted: Vec<u64> = (cursor.saturating_sub(1)..(cursor + 2).min(photos.len()))
            .map(|i| photos[i].id)
            .collect();
        self.full.retain(|id, _| wanted.contains(id));
        for id in wanted {
            if self.full.contains_key(&id) || self.loading.contains(&id) {
                continue;
            }
            let Some(file) = self.reel.get(id).and_then(|p| p.file.clone()) else {
                continue;
            };
            self.loading.insert(id);
            spawn(&self.tx, move || Done::Full(id, reel::load(&file, Some(side))));
        }
    }
}

impl App {
    /// Every frame: the workers' thumbnails, copies, pictures and photos
    /// read back.
    pub(super) fn photo_reel_update(&mut self) {
        while let Ok(done) = self.snapshots.reel.rx.try_recv() {
            let r = &mut self.snapshots.reel;
            match done {
                Done::Thumb(id, (w, h, px)) => {
                    if r.reel.get(id).is_some() {
                        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                        let tex = self
                            .egui_ctx
                            .load_texture(format!("photo_reel_{id}"), image, egui::TextureOptions::LINEAR);
                        r.thumbs.insert(id, tex);
                    }
                }
                Done::Copied(id, Ok(path)) => {
                    if r.reel.get(id).is_some_and(|p| p.own_copy) {
                        r.reel.copied(id, path);
                    } else {
                        // gone meanwhile
                        delete_files(vec![path]);
                    }
                }
                Done::Copied(id, Err(e)) => {
                    log::warn!("photo reel: copy of photo {id} not written: {e}");
                    r.reel.copy_failed(id);
                }
                Done::Full(id, result) => {
                    r.loading.remove(&id);
                    match result {
                        Ok((w, h, px)) if r.reel.get(id).is_some() => {
                            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                            let tex = self
                                .egui_ctx
                                .load_texture(format!("photo_reel_full_{id}"), image, egui::TextureOptions::LINEAR);
                            r.full.insert(id, tex);
                        }
                        Ok(_) => {}
                        Err(e) => log::warn!("photo reel: photo {id} not read back: {e}"),
                    }
                }
                Done::Held(id, destination, window, result) => {
                    if r.holding != Some(id) {
                        continue;
                    }
                    r.holding = None;
                    match result {
                        Ok(pixels) => self.photo_reel_hold(id, destination, pixels, window),
                        Err(e) => {
                            log::warn!("photo reel: photo {id} not read back: {e}");
                            let text = aurora_i18n::tr!("photo-failed", error = e.as_str()).to_owned();
                            self.photo_reel_say(text, true);
                        }
                    }
                }
                // the photo window's latest photos read again without them
                Done::Recycled(Ok(())) => self.snapshots.gallery_reread(),
                Done::Recycled(Err(e)) => {
                    log::warn!("photo reel: not sent to the Recycle Bin: {e}");
                    let text = aurora_i18n::tr!("photo-reel-delete-failed", error = e.as_str()).to_owned();
                    self.photo_reel_say(text, true);
                }
            }
        }
    }

    /// The last photo's card: its thumbnail, from the reel.
    pub(super) fn photo_reel_card(&self) -> Option<egui::TextureHandle> {
        let r = &self.snapshots.reel;
        r.thumbs.get(&r.card?).cloned()
    }

    /// The destinations a photo of the reel can be sent to now: none while
    /// one is sent, each as its own button in the photo window allows it.
    pub(super) fn photo_reel_destinations(&self, window: (u32, u32)) -> Vec<Destination> {
        if self.photo_busy() || self.snapshots.reel.holding.is_some() {
            return Vec::new();
        }
        let mut out = vec![Destination::Disk];
        let inv = self.photo_inventory_view(window);
        if inv.connected && inv.cost.is_some() {
            out.push(Destination::Inventory);
        }
        let profile = self.photo_profile_view(window);
        if profile.connected {
            out.push(Destination::Profile);
        }
        if self.photo_email_view(window).available {
            out.push(Destination::Email);
        }
        out
    }

    /// The quick view over the photo mode, while it is open.
    pub(super) fn photo_reel_quick(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, renderer: &Renderer) {
        if !self.snapshots.reel.quick.is_open() {
            return;
        }
        let window = renderer.size();
        self.snapshots.reel.read_ahead(window.0.max(window.1));
        let can_send = self.photo_reel_destinations(window);
        let island = self.settings.system_island;
        let time = move |t: i64| crate::ui::bars::clock::time_of(island, t);
        let mut quick = std::mem::take(&mut self.snapshots.reel.quick);
        quick.blocked = self.snapshots.reel.mail.is_some()
            || self.snapshots.reel.apply.is_some()
            || self.snapshots.confirm.is_some()
            || self.snapshots.short.is_some();
        let mut out = Vec::new();
        quick.show(ctx, p, &self.snapshots.reel.view(&can_send, &time), &mut out);
        self.snapshots.reel.quick = quick;
        for a in out {
            self.photo_reel_action(a, renderer);
        }
        // the dialogs of a send, over it: the price, the e-mail's form
        let dialogs = self.snapshots.ui.dialogs(ctx, p, self.snapshots.confirm, self.snapshots.short);
        for a in dialogs {
            self.photo_action(a, renderer);
        }
        self.photo_reel_mail_form(ctx, p, window);
        self.photo_reel_apply_form(ctx, p);
    }

    /// The toolbar's Photo roll: the quick view on the newest photo shown
    /// (not one still on its way), in photo mode or without it; closed
    /// when open, like the toolbar's windows.
    pub(super) fn photo_reel_show_newest(&mut self) {
        let r = &mut self.snapshots.reel;
        if r.quick.is_open() {
            r.quick.close();
            r.mail = None;
            r.apply = None;
            return;
        }
        let Some(id) = r.view(&[], &|_| String::new()).photos.first().map(|p| p.id) else {
            return;
        };
        r.quick.open = Some(id);
        r.in_photo_mode = self.panels.snapshot;
    }

    /// Our profile on its main tab, where its picture shows; the quick view
    /// closed (it covers the windows) and photo mode left, the reel kept.
    pub(super) fn photo_reel_action_show_profile(&mut self) {
        self.snapshots.reel.quick.close();
        self.snapshots.reel.mail = None;
        self.snapshots.reel.apply = None;
        self.panels.snapshot = false;
        self.profile_ui.open_main(&mut self.world);
    }

    /// What the reel's stack or quick view asks.
    pub(super) fn photo_reel_action(&mut self, a: ReelAction, renderer: &Renderer) {
        match a {
            ReelAction::Show(id) => {
                self.snapshots.reel.quick.open = Some(id);
                self.snapshots.reel.in_photo_mode = self.panels.snapshot;
            }
            ReelAction::Close => {
                self.snapshots.reel.quick.close();
                self.snapshots.reel.mail = None;
                self.snapshots.reel.apply = None;
            }
            // an e-mail asks its recipients first, over the quick view
            ReelAction::Send(id, Destination::Email) => self.snapshots.reel.mail = Some(id),
            // the profile picture is accepted first
            ReelAction::Send(id, Destination::Profile) => self.snapshots.reel.apply = Some(id),
            ReelAction::Send(id, destination) => self.photo_reel_send(id, destination, renderer.size()),
            ReelAction::Reveal(id) => {
                if let Some(path) = self.snapshots.reel.reel.get(id).and_then(|p| p.on_disk()) {
                    let folder = path.parent().map(PathBuf::from).unwrap_or_default();
                    crate::snapshot::gallery::reveal(&folder, Some(path));
                }
            }
            ReelAction::ShowItem(id) => {
                if let Some(item) = self.snapshots.reel.reel.get(id).and_then(|p| p.item()) {
                    self.snapshots.reel.quick.close();
                    // photo mode left too: the inventory takes over
                    self.snapshots.reel.mail = None;
                    self.snapshots.reel.apply = None;
                    self.panels.snapshot = false;
                    self.panels.inventory = true;
                    self.inventory_ui.show_original(&self.world.inventory, item);
                }
            }
            ReelAction::ShowProfile => self.photo_reel_action_show_profile(),
            ReelAction::Delete(id) => self.photo_reel_delete(id),
            // the photos' folder, the last photo saved selected in it
            ReelAction::OpenFolder => self.photo_action(crate::ui::snapshot::Action::OpenFolder, renderer),
        }
    }

    /// Read the photo back on a worker, fitted to the destination (a disk
    /// photo whole, an inventory one at its own size), to hold it.
    fn photo_reel_send(&mut self, id: u64, destination: Destination, window: (u32, u32)) {
        let Some(photo) = self.snapshots.reel.reel.get(id) else {
            return;
        };
        let Some(file) = photo.file.clone() else {
            return;
        };
        let size = photo.size;
        let side = match destination {
            Destination::Disk => None,
            Destination::Inventory => Some(inventory::reel_side(size)),
            _ => {
                let (w, h) = self.photo_size_for(destination, window);
                let long = w.max(h);
                let (w, h) = reel::fit_inside(size, (long, long));
                Some(w.max(h))
            }
        };
        self.snapshots.reel.holding = Some(id);
        spawn(&self.snapshots.reel.tx, move || {
            let result = reel::load(&file, side);
            Done::Held(id, destination, window, result)
        });
    }

    /// The photo read back: sent at once to its destination, from the quick
    /// view (nothing held, the view, the window's destination and the
    /// quick view unchanged). The inventory's price is confirmed first, as
    /// for a new photo (LLPanelSnapshotInventory::onSend).
    fn photo_reel_hold(&mut self, id: u64, destination: Destination, (w, h, rgba): Pixels, window: (u32, u32)) {
        let Some(cutout) = self.snapshots.reel.reel.get(id).map(|p| p.cutout) else {
            return;
        };
        if self.photo_busy() {
            return;
        }
        if destination == Destination::Inventory {
            // (its price follows its size: photo_inventory_view)
            self.snapshots.reel.pending = Some((id, (w, h, rgba)));
            let view = self.photo_inventory_view(window);
            let Some(cost) = view.cost.filter(|_| view.connected) else {
                self.snapshots.reel.pending = None;
                return;
            };
            if self.world.balance.is_some_and(|b| b < cost) {
                self.snapshots.reel.pending = None;
                self.snapshots.short = Some(cost);
            } else if cost > 0 {
                self.snapshots.confirm = Some(cost);
            } else {
                self.photo_reel_confirmed(Some(cost));
            }
            return;
        }
        // (the destination follows this photo of the reel, not a new one)
        let before = self.snapshots.reel.held.replace(id);
        match destination {
            Destination::Disk => self.photo_save_pixels((w, h), rgba, cutout),
            Destination::Email => self.photo_postcard((w, h), rgba, cutout),
            // (the inventory: above; the profile takes the uploaded item)
            Destination::Inventory | Destination::Profile => {}
        }
        self.snapshots.reel.held = before;
    }

    /// The price confirmed (`Some`) or not: the reel's photo waiting for it
    /// uploaded or let go. False when none waits (the confirmation was a
    /// new photo's).
    pub(super) fn photo_reel_confirmed(&mut self, cost: Option<i32>) -> bool {
        let Some((id, (w, h, rgba))) = self.snapshots.reel.pending.take() else {
            return false;
        };
        let cutout = self.snapshots.reel.reel.get(id).map(|p| p.cutout);
        if let (Some(cost), Some(cutout)) = (cost, cutout)
            && !self.photo_busy()
        {
            let before = self.snapshots.reel.held.replace(id);
            self.photo_encode((w, h), rgba, cost, cutout);
            self.snapshots.reel.held = before;
        }
        true
    }

    /// "Use this photo as your profile picture?" over the quick view, for a
    /// photo uploaded to the inventory: accepted, its texture becomes our
    /// profile's picture (LLPanelProfileSecondLife::onCommitProfileImage,
    /// sl_image_id); a refusal of the server is said by ProfileSaveFailed.
    fn photo_reel_apply_form(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(id) = self.snapshots.reel.apply else {
            return;
        };
        let modal = crate::ui::widgets::Modal::new(egui::Id::unique("photo_reel_apply")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("photo-reel-apply-title"))
                    .strong()
                    .size(15.0)
                    .color(p.ink),
            );
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(aurora_i18n::tr!("photo-reel-apply-text")).color(p.ink)).wrap());
            ui.add_space(10.0);
            crate::ui::widgets::dialog_buttons(
                ui,
                p,
                &[
                    crate::ui::widgets::DialogButton {
                        icon: "",
                        label: aurora_i18n::tr!("common-cancel"),
                        kind: crate::ui::widgets::ActionKind::Flat,
                        enabled: true,
                    },
                    crate::ui::widgets::DialogButton {
                        icon: "user-filled",
                        label: aurora_i18n::tr!("photo-reel-apply-ok"),
                        kind: crate::ui::widgets::ActionKind::Primary,
                        enabled: true,
                    },
                ],
                32.0,
            )
        });
        // (over the quick view, drawn before it)
        ctx.move_to_top(modal.backdrop_response.layer_id);
        ctx.move_to_top(modal.response.layer_id);
        match modal.inner {
            Some(1) => {
                self.snapshots.reel.apply = None;
                self.photo_reel_apply(id);
            }
            Some(_) => self.snapshots.reel.apply = None,
            None if modal.should_close() => self.snapshots.reel.apply = None,
            None => {}
        }
    }

    /// The photo's uploaded texture becomes our profile picture, and our
    /// profile shows it.
    pub(super) fn photo_reel_apply(&mut self, id: u64) {
        let asset = self
            .snapshots
            .reel
            .reel
            .get(id)
            .and_then(|photo| photo.item())
            .and_then(|item| self.world.inventory.items.get(&item))
            .map(|item| item.asset_id)
            .filter(|asset| !asset.is_nil());
        let Some(asset) = asset else {
            log::warn!("photo reel: photo {id} has no uploaded texture to apply");
            self.photo_reel_say(aurora_i18n::tr!("photo-reel-apply-failed").to_owned(), true);
            return;
        };
        log::info!("photo reel: photo {id} applied as the profile picture");
        // (the demo's upload keeps no pixels for the grid to send back)
        if self.demo
            && let Some(thumb) = self.snapshots.reel.thumbs.get(&id)
        {
            self.avatar_pic_images.insert(asset, thumb.clone());
        }
        let mut data = aurora_llsd::Llsd::new_map();
        data.insert("sl_image_id", asset);
        let me = self.world.agent_id;
        let command = self.world.save_profile(me, data);
        self.send(command);
        self.snapshots.reel.reel.sent(id, Place::Profile);
        self.photo_reel_action_show_profile();
    }

    /// The postcard's form over the quick view, for a photo of the reel:
    /// the photo window's own fields (kept between them), sent with the
    /// recipients checked (PromptRecipientEmail), or cancelled.
    fn photo_reel_mail_form(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, window: (u32, u32)) {
        let Some(id) = self.snapshots.reel.mail else {
            return;
        };
        if self.snapshots.reel.reel.get(id).is_none() {
            self.snapshots.reel.mail = None;
            self.snapshots.reel.apply = None;
            return;
        }
        let mail = self.photo_email_view(window);
        let mut answer = None;
        let modal = crate::ui::widgets::Modal::new(egui::Id::unique("photo_reel_mail")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("photo-reel-email"))
                    .strong()
                    .size(15.0)
                    .color(p.ink),
            );
            ui.add_space(6.0);
            self.snapshots.ui.postcard_form(ui, p, &mut self.settings.snapshot, &mail);
            ui.add_space(10.0);
            let valid = aurora_net::postcard::valid_recipients(&self.snapshots.ui.postcard.to);
            ui.horizontal(|ui| {
                let size = egui::Vec2::new(0.0, 28.0);
                let send = crate::ui::widgets::action_button_if(
                    ui,
                    p,
                    "mail-filled",
                    aurora_i18n::tr!("photo-email-send"),
                    crate::ui::widgets::ActionKind::Primary,
                    size,
                    valid && mail.available,
                );
                if send.clicked() {
                    answer = Some(true);
                }
                if crate::ui::widgets::action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("common-cancel"),
                    crate::ui::widgets::ActionKind::Flat,
                    size,
                )
                .clicked()
                {
                    answer = Some(false);
                }
            });
        });
        // (over the quick view, drawn before it)
        ctx.move_to_top(modal.backdrop_response.layer_id);
        ctx.move_to_top(modal.response.layer_id);
        if answer.is_none() && modal.should_close() {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.snapshots.reel.mail = None;
                self.snapshots.reel.apply = None;
                self.photo_reel_send(id, Destination::Email, window);
            }
            Some(false) => self.snapshots.reel.mail = None,
            None => {}
        }
    }

    /// Delete a photo, confirmed: its files to the Recycle Bin on a worker,
    /// its Photo Album item to the inventory's Trash, the reel's copy.
    fn photo_reel_delete(&mut self, id: u64) {
        let Some(photo) = self.snapshots.reel.reel.get(id) else {
            return;
        };
        let d = reel::deletion(photo);
        for item in &d.trash {
            let inv = &self.world.inventory;
            let moved = crate::world::inventory::actions::system(inv, 14)
                .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
                .and_then(|trash| {
                    crate::world::inventory::actions::move_selection(
                        inv,
                        &[*item],
                        trash,
                        &self.settings.inventory.protected,
                        &HashSet::new(),
                    )
                });
            match moved {
                Ok(change) => self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(change)),
                Err(reason) => {
                    log::warn!("photo reel: item {item} not moved to the Trash: {reason}");
                    let text = aurora_i18n::tr!("photo-reel-delete-failed", error = reason.as_str()).to_owned();
                    self.photo_reel_say(text, true);
                }
            }
        }
        if !d.recycle.is_empty() {
            let files = d.recycle.clone();
            spawn(&self.snapshots.reel.tx, move || {
                Done::Recycled(trash::delete_all(&files).map_err(|e| e.to_string()))
            });
        }
        log::info!(
            "photo reel: photo {id} deleted ({} file(s) to the Recycle Bin, {} item(s) to the Trash)",
            d.recycle.len(),
            d.trash.len()
        );
        let r = &mut self.snapshots.reel;
        let cursor = r.reel.index_of(id).unwrap_or(0);
        if let Some(photo) = r.reel.remove(id) {
            r.forget(vec![photo]);
        }
        // the quick view moves to the next older photo, or closes
        if r.quick.open == Some(id) {
            r.quick.open = reel::after_removal(r.reel.len(), cursor).map(|i| r.reel.photos()[i].id);
        }
        self.photo_reel_say(aurora_i18n::tr!("photo-reel-deleted").to_owned(), false);
    }

    /// A line of the photo mode: the viewfinder's, the photo window's.
    fn photo_reel_say(&mut self, text: String, error: bool) {
        let now = self.egui_ctx.input(|i| i.time);
        self.snapshots.vf.say(text.clone(), error, now);
        self.snapshots.reel_message(text, error);
    }

    /// The session is over: the reel empties, its folder goes.
    pub(super) fn photo_reel_end(&mut self) {
        let r = &mut self.snapshots.reel;
        r.quick.close();
        let photos = r.reel.clear();
        r.forget(photos);
        r.sending = None;
        r.pending = None;
        r.mail = None;
        r.apply = None;
        r.holding = None;
        r.loading.clear();
        // (dropped: its folder is deleted)
        r.folder = None;
    }
}
