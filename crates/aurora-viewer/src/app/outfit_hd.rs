//! The outfit's paid HD picture on the app's side (Fider #447, the user's
//! choice of 2026-11-10; the rules are `snapshot::portrait`). Besides the
//! free 256 × 256 thumbnail, the confirmed portrait can also be sent as a
//! texture into the outfit's own folder, at the grid's texture upload price
//! (`texture_upload_cost`), shown and confirmed before anything is paid. The
//! showcase's large preview shows it; the cards keep the thumbnail.
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
use std::collections::HashSet;
use uuid::Uuid;

/// The encoded texture and the pixels it was made from (shown at once in
/// the showcase once it is saved, instead of being downloaded again).
type Encoded = Result<(Vec<u8>, Vec<u8>), String>;

enum Step {
    Encoding(crossbeam_channel::Receiver<Encoded>),
    Sent { request: Uuid, pixels: Vec<u8> },
}

struct Upload {
    outfit: Uuid,
    side: u32,
    cost: i32,
    step: Step,
}

/// The HD picture on its way, and the older ones waiting for the Trash.
#[derive(Default)]
pub(super) struct OutfitHd {
    upload: Option<Upload>,
    /// Moved once no other inventory change is under way.
    trash: Vec<Uuid>,
}

impl App {
    /// The HD picture of this side, now: its price and whether the balance
    /// pays it.
    pub(super) fn outfit_hd_offer(&self, side: u32) -> HdOffer {
        portrait::hd_offer(self.photo_prices().cost(Kind::Texture, (side, side)), self.world.balance)
    }

    /// An HD picture is being encoded or sent.
    pub(super) fn outfit_hd_busy(&self) -> bool {
        self.inventory_images.hd.upload.is_some()
    }

    /// The confirmed photo (square, `size`) as the outfit's HD picture of
    /// `side`, at the confirmed price: reduced and encoded on a worker.
    pub(super) fn outfit_hd_start(&mut self, outfit: Uuid, size: (u32, u32), pixels: Vec<u8>, side: u32, cost: i32) {
        if self.outfit_hd_busy() {
            return;
        }
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
                    inventory::encode(side, side, &rgba, (side, side)).map(|data| (data, rgba))
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
        log::info!("outfit HD picture: {side} x {side} for {outfit}, L$ {cost} confirmed");
        self.inventory_images.hd.upload = Some(Upload {
            outfit,
            side,
            cost,
            step: Step::Encoding(rx),
        });
        self.appearance_ui.message = aurora_i18n::tr!("app-outfit-hd-sending", cost = cost).to_owned();
    }

    /// Every frame: the encoded picture is sent; the older ones go to the
    /// Trash once the inventory is free.
    pub(super) fn poll_outfit_hd(&mut self) {
        self.outfit_hd_send();
        self.outfit_hd_trash();
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
        let (outfit, cost) = (up.outfit, up.cost);
        let (data, pixels) = match encoded {
            Ok(done) => done,
            Err(error) => {
                log::warn!("outfit HD picture: not encoded: {error}");
                self.inventory_images.hd.upload = None;
                self.appearance_ui.message = aurora_i18n::tr!("app-outfit-hd-failed", error = error).to_owned();
                return;
            }
        };
        let inv = &self.world.inventory;
        let Some(name) = inv.folders.get(&outfit).map(|f| f.info.name.clone()) else {
            self.inventory_images.hd.upload = None;
            self.appearance_ui.message = aurora_i18n::tr!("app-outfit-hd-outfit-gone").to_owned();
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
                let image = egui::ColorImage::from_rgba_unmultiplied([up.side as usize, up.side as usize], &pixels);
                let texture = self
                    .egui_ctx
                    .load_texture(format!("img-{asset}"), image, egui::TextureOptions::LINEAR);
                self.ui_images.insert(asset, texture);
                log::info!(
                    "outfit HD picture: saved for L$ {}, {} older one(s) to the Trash",
                    receipt.cost,
                    older.len()
                );
                self.inventory_images.hd.trash.extend(older);
                self.appearance_ui.message = aurora_i18n::tr!("app-outfit-hd-saved", cost = receipt.cost).to_owned();
            }
            Err(e) => {
                log::warn!("outfit HD picture: upload failed: {e}");
                self.appearance_ui.message = match e {
                    upload::Error::PriceChanged(Some(cost)) => aurora_i18n::tr!("app-outfit-hd-price-changed", cost = cost).to_owned(),
                    upload::Error::Unknown | upload::Error::Uncertain(_) | upload::Error::Verification => {
                        aurora_i18n::tr!("app-outfit-hd-unknown").to_owned()
                    }
                    upload::Error::Rejected | upload::Error::Refused(_) => aurora_i18n::tr!("app-outfit-hd-refused").to_owned(),
                    other => aurora_i18n::tr!("app-outfit-hd-failed", error = other.to_string()).to_owned(),
                };
            }
        }
    }
}
