//! The outfit's paid HD picture on the app's side (Fider #447, the user's
//! choice of 2026-11-10; the rules are `snapshot::portrait`). Besides the
//! free 256 × 256 thumbnail, the confirmed portrait can also be sent as a
//! texture into the outfit's own folder, at the grid's texture upload price
//! (`texture_upload_cost`), shown and confirmed before anything is paid. The
//! showcase's large preview shows it; the cards keep the thumbnail.
//!
//! The batch portraits (`outfit_batch`) confirm the total once, then queue
//! one picture an outfit here: they are sent one after the other, each at
//! the price confirmed, and counted for the batch's summary instead of
//! taking the gallery's message, which the batch reads for its own photos.
//!
//! One HD picture an outfit, as LLAppearanceMgr::removeOutfitPhoto kept one
//! photo: once the new one is saved, the older ones this viewer sent go to
//! the Trash (never deleted: a paid texture stays recoverable). A texture
//! the user put in the folder themselves is never touched.
//!
//! The asset id is never shown: the picture is an item of the user's own
//! folder, read like the gallery reads the others.

use super::App;
use crate::snapshot::inventory;
use crate::snapshot::portrait::{self, HdOffer};
use aurora_net::NetCommand;
use aurora_net::inventory::upload::{self, Kind, Receipt};
use std::collections::{HashSet, VecDeque};
use uuid::Uuid;

/// The encoded texture and the pixels it was made from (shown at once in
/// the showcase once it is saved, instead of being downloaded again; not
/// kept for a batch's pictures).
type Encoded = Result<(Vec<u8>, Vec<u8>), String>;

enum Step {
    Encoding(crossbeam_channel::Receiver<Encoded>),
    Sent { request: Uuid, pixels: Vec<u8> },
}

/// A confirmed picture waiting for its turn.
struct Job {
    outfit: Uuid,
    size: (u32, u32),
    pixels: Vec<u8>,
    side: u32,
    cost: i32,
    batch: bool,
}

struct Upload {
    outfit: Uuid,
    side: u32,
    cost: i32,
    batch: bool,
    step: Step,
}

/// The batch's HD pictures so far (`outfit_batch`'s summary).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BatchTally {
    pub saved: usize,
    pub paid: i32,
    pub failed: usize,
    /// The last failure's reason.
    pub error: Option<String>,
}

/// The HD picture on its way, those waiting, and the older ones waiting
/// for the Trash.
#[derive(Default)]
pub(super) struct OutfitHd {
    upload: Option<Upload>,
    queue: VecDeque<Job>,
    /// Moved once no other inventory change is under way.
    trash: Vec<Uuid>,
    pub batch: BatchTally,
}

impl OutfitHd {
    /// The batch's pictures still encoded, sent or waiting.
    pub(super) fn batch_pending(&self) -> usize {
        self.queue.iter().filter(|j| j.batch).count() + usize::from(self.upload.as_ref().is_some_and(|u| u.batch))
    }
}

impl App {
    /// The HD picture of this side, now: its price and whether the balance
    /// pays it.
    pub(super) fn outfit_hd_offer(&self, side: u32) -> HdOffer {
        portrait::hd_offer(self.outfit_hd_price(side), self.world.balance)
    }

    /// The price of one HD picture of this side (None: not known).
    pub(super) fn outfit_hd_price(&self, side: u32) -> Option<i32> {
        self.photo_prices().cost(Kind::Texture, (side, side))
    }

    /// An HD picture is being encoded or sent, or waits for its turn.
    pub(super) fn outfit_hd_busy(&self) -> bool {
        let hd = &self.inventory_images.hd;
        hd.upload.is_some() || !hd.queue.is_empty()
    }

    /// The confirmed photo (square, `size`) as the outfit's HD picture of
    /// `side`, at the confirmed price: reduced and encoded on a worker, or
    /// queued behind the one on its way (`batch`: one of the batch
    /// portraits', counted for its summary).
    pub(super) fn outfit_hd_start(&mut self, outfit: Uuid, size: (u32, u32), pixels: Vec<u8>, side: u32, cost: i32, batch: bool) {
        log::info!("outfit HD picture: {side} x {side} for {outfit}, L$ {cost} confirmed");
        let hd = &mut self.inventory_images.hd;
        hd.queue.push_back(Job {
            outfit,
            size,
            pixels,
            side,
            cost,
            batch,
        });
        if !batch {
            self.appearance_ui.message = aurora_i18n::tr!("app-outfit-hd-sending", cost = cost).to_owned();
        }
        self.outfit_hd_next();
    }

    /// The next waiting picture, once none is on its way.
    fn outfit_hd_next(&mut self) {
        let hd = &mut self.inventory_images.hd;
        if hd.upload.is_some() {
            return;
        }
        let Some(job) = hd.queue.pop_front() else {
            return;
        };
        let Job {
            outfit,
            size,
            pixels,
            side,
            cost,
            batch,
        } = job;
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let encoded = image::RgbaImage::from_raw(size.0, size.1, pixels)
                .ok_or_else(|| "photo pixels".to_owned())
                .map(|image| {
                    if image.dimensions() == (side, side) {
                        image
                    } else {
                        image::imageops::resize(&image, side, side, image::imageops::FilterType::Lanczos3)
                    }
                })
                .and_then(|image| {
                    let rgba = image.into_raw();
                    inventory::encode(side, side, &rgba, (side, side)).map(|data| (data, if batch { Vec::new() } else { rgba }))
                });
            if let Ok((data, _)) = &encoded {
                log::info!(
                    "outfit HD picture: {side} x {side} encoded in {} ms ({} KB)",
                    t0.elapsed().as_millis(),
                    data.len() >> 10
                );
            }
            let _ = tx.send(encoded);
        });
        hd.upload = Some(Upload {
            outfit,
            side,
            cost,
            batch,
            step: Step::Encoding(rx),
        });
    }

    /// Every frame: the encoded picture is sent, the next one started; the
    /// older ones go to the Trash once the inventory is free.
    pub(super) fn poll_outfit_hd(&mut self) {
        self.outfit_hd_send();
        self.outfit_hd_next();
        self.outfit_hd_trash();
    }

    /// A picture that will not be sent: the gallery's message, or the
    /// batch's summary.
    fn outfit_hd_failed(&mut self, batch: bool, text: String) {
        if batch {
            let tally = &mut self.inventory_images.hd.batch;
            tally.failed += 1;
            tally.error = Some(text);
        } else {
            self.appearance_ui.message = text;
        }
    }

    fn outfit_hd_send(&mut self) {
        let Some(up) = &self.inventory_images.hd.upload else {
            return;
        };
        let Step::Encoding(rx) = &up.step else {
            return;
        };
        let Ok(encoded) = rx.try_recv() else {
            return;
        };
        let (outfit, cost, batch) = (up.outfit, up.cost, up.batch);
        let (data, pixels) = match encoded {
            Ok(done) => done,
            Err(error) => {
                log::warn!("outfit HD picture: not encoded: {error}");
                self.inventory_images.hd.upload = None;
                self.outfit_hd_failed(batch, aurora_i18n::tr!("app-outfit-hd-failed", error = error).to_owned());
                return;
            }
        };
        let inv = &self.world.inventory;
        let Some(name) = inv.folders.get(&outfit).map(|f| f.info.name.clone()) else {
            self.inventory_images.hd.upload = None;
            self.outfit_hd_failed(batch, aurora_i18n::tr!("app-outfit-hd-outfit-gone").to_owned());
            return;
        };
        let request = Uuid::new_v4();
        let upload = upload::Request {
            kind: Kind::Texture,
            folder: outfit,
            name: portrait::hd_picture_name(&name),
            description: portrait::HD_PICTURE_DESC.to_owned(),
            // the uploads' permissions (LLFloaterPerms "Uploads")
            permissions: self.settings.imports.permissions(),
            expected_cost: cost,
            data,
        };
        log::info!(
            "outfit HD picture: upload of {} bytes into the outfit for L$ {cost}",
            upload.data.len()
        );
        if let Some(up) = &mut self.inventory_images.hd.upload {
            up.step = Step::Sent { request, pixels };
        }
        self.send(NetCommand::UploadAsset { request, upload });
    }

    fn outfit_hd_trash(&mut self) {
        let hd = &mut self.inventory_images.hd;
        if hd.trash.is_empty() || self.inventory_ui.pending.is_some() || self.appearance_ui.pending.is_some() {
            return;
        }
        let inv = &self.world.inventory;
        let items: Vec<Uuid> = std::mem::take(&mut hd.trash)
            .into_iter()
            .filter(|id| inv.items.contains_key(id))
            .collect();
        if items.is_empty() {
            return;
        }
        let moved = crate::world::inventory::actions::system(inv, 14)
            .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
            .and_then(|trash| {
                crate::world::inventory::actions::move_selection(inv, &items, trash, &self.settings.inventory.protected, &HashSet::new())
            });
        match moved {
            Ok(change) => {
                log::info!("outfit HD picture: {} older one(s) moved to the Trash", items.len());
                self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(change));
            }
            Err(reason) => log::warn!("outfit HD picture: older ones not moved to the Trash: {reason}"),
        }
    }

    /// The upload this request belongs to is the HD picture's.
    pub(super) fn outfit_hd_owns(&self, request: Uuid) -> bool {
        matches!(&self.inventory_images.hd.upload, Some(Upload { step: Step::Sent { request: r, .. }, .. }) if *r == request)
    }

    /// The server's answer to the HD picture's upload.
    pub(super) fn outfit_hd_uploaded(&mut self, result: Result<Receipt, upload::Error>) {
        let Some(up) = self.inventory_images.hd.upload.take() else {
            return;
        };
        let Step::Sent { pixels, .. } = up.step else {
            return;
        };
        match result {
            Ok(receipt) => {
                let mut item = receipt.item;
                if item.created_at == 0 {
                    // (the newest is the one shown; the demo's has no date)
                    item.created_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64);
                }
                let (id, asset) = (item.id, item.asset_id);
                let older: Vec<Uuid> = crate::world::appearance::hd_pictures(&self.world.inventory, up.outfit)
                    .iter()
                    .map(|it| it.id)
                    .filter(|old| *old != id)
                    .collect();
                self.world.inventory.add_items(vec![item]);
                // (a batch's pictures are fetched when shown, rather than
                // each kept on the GPU)
                if !pixels.is_empty() {
                    let image = egui::ColorImage::from_rgba_unmultiplied([up.side as usize, up.side as usize], &pixels);
                    let texture = self
                        .egui_ctx
                        .load_texture(format!("img-{asset}"), image, egui::TextureOptions::LINEAR);
                    self.ui_images.insert(asset, texture);
                }
                log::info!(
                    "outfit HD picture: saved for L$ {}, {} older one(s) to the Trash",
                    receipt.cost,
                    older.len()
                );
                self.inventory_images.hd.trash.extend(older);
                if up.batch {
                    let tally = &mut self.inventory_images.hd.batch;
                    tally.saved += 1;
                    tally.paid += receipt.cost;
                } else {
                    self.appearance_ui.message = aurora_i18n::tr!("app-outfit-hd-saved", cost = receipt.cost).to_owned();
                }
            }
            Err(e) => {
                log::warn!("outfit HD picture: upload failed: {e}");
                if up.batch && matches!(e, upload::Error::PriceChanged(_)) {
                    // the price the batch confirmed no longer holds: the
                    // batch's others are not sent at it either
                    let hd = &mut self.inventory_images.hd;
                    let before = hd.queue.len();
                    hd.queue.retain(|j| !j.batch);
                    let dropped = before - hd.queue.len();
                    hd.batch.failed += dropped;
                    log::warn!("outfit HD picture: the price changed, {dropped} more of the batch not sent");
                }
                let text = match e {
                    upload::Error::PriceChanged(Some(cost)) => aurora_i18n::tr!("app-outfit-hd-price-changed", cost = cost).to_owned(),
                    upload::Error::Unknown | upload::Error::Uncertain(_) | upload::Error::Verification => {
                        aurora_i18n::tr!("app-outfit-hd-unknown").to_owned()
                    }
                    upload::Error::Rejected | upload::Error::Refused(_) => aurora_i18n::tr!("app-outfit-hd-refused").to_owned(),
                    other => aurora_i18n::tr!("app-outfit-hd-failed", error = other.to_string()).to_owned(),
                };
                self.outfit_hd_failed(up.batch, text);
            }
        }
        self.outfit_hd_next();
    }
}
