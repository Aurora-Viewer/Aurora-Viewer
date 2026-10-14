//! The demo's Marketplace listings (AURORA_DEMO_MARKETPLACE_LISTINGS, see
//! docs/TESTING.md): an SLM server simulated in the viewer and a sample
//! listings folder. Nothing leaves the viewer and no grid data is read.

use super::{LISTINGS, STOCK};
use crate::world::inventory::Inventory;
use crate::world::inventory::actions::{COPY, MODIFY, TRANSFER};
use aurora_net::inventory::{InvFolder, InvItem};
use aurora_net::slm::{Listing, MerchantStatus, SlmError, SlmReply, SlmRequest};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A merchant with three listings.
    Merchant,
    /// GET /merchant answers 404: Firestorm's explanation and its link.
    NotMerchant,
    /// GET /merchant fails: the connection error, with its reason.
    Error,
}

impl Mode {
    pub fn from_env() -> Option<Mode> {
        match std::env::var("AURORA_DEMO_MARKETPLACE_LISTINGS").ok()?.as_str() {
            "not-merchant" => Some(Mode::NotMerchant),
            "error" => Some(Mode::Error),
            "" => None,
            _ => Some(Mode::Merchant),
        }
    }
}

pub const HOODIE: Uuid = Uuid::from_u128(8_400_010);
pub const HOODIE_V1: Uuid = Uuid::from_u128(8_400_011);
pub const HOODIE_V2: Uuid = Uuid::from_u128(8_400_012);
pub const POSTER: Uuid = Uuid::from_u128(8_400_020);
pub const POSTER_VERSION: Uuid = Uuid::from_u128(8_400_021);
pub const POSTER_STOCK: Uuid = Uuid::from_u128(8_400_022);
pub const LANTERN: Uuid = Uuid::from_u128(8_400_030);
pub const DRAFT: Uuid = Uuid::from_u128(8_400_040);
pub const DRAFT_VERSION: Uuid = Uuid::from_u128(8_400_041);
pub const PRODUCTS: Uuid = Uuid::from_u128(8_400_050);
const DEMO_ROOT: Uuid = Uuid::from_u128(8_400_000);

/// The simulated SLM server.
pub struct DemoSlm {
    pub mode: Mode,
    /// The sample listings are in the inventory.
    pub seeded: bool,
    listings: Vec<Listing>,
    next: i32,
}

fn edit_url(id: i32) -> String {
    // the Marketplace home, so the demo never opens an invented page
    format!("https://marketplace.secondlife.com/#demo-listing-{id}")
}

impl DemoSlm {
    pub fn new(mode: Mode) -> Self {
        let listing = |id, folder, version, is_listed, count| Listing {
            id,
            is_listed,
            edit_url: edit_url(id),
            folder,
            version,
            count,
        };
        DemoSlm {
            mode,
            seeded: false,
            listings: vec![
                listing(4101, HOODIE, HOODIE_V2, true, -1),
                listing(4102, POSTER, POSTER_VERSION, false, 3),
                listing(4103, DRAFT, DRAFT_VERSION, false, -1),
            ],
            next: 4104,
        }
    }

    /// GET /merchant.
    pub fn merchant(&self) -> MerchantStatus {
        match self.mode {
            Mode::Merchant => MerchantStatus::Merchant,
            Mode::NotMerchant => MerchantStatus::NotMerchant,
            Mode::Error => MerchantStatus::ConnectionFailure("503 Service Unavailable (demo)".into()),
        }
    }

    /// What SLM answers, from the demo listings. Listing the draft fails
    /// like an incomplete listing does (422), and a listing without stock
    /// stays unlisted (SLM keeps it out of sale).
    pub fn answer(&mut self, request: &SlmRequest) -> SlmReply {
        let not_found = || {
            SlmReply::Failed(SlmError::Transaction {
                reason: "'Not Found'\n".into(),
                description: String::new(),
            })
        };
        match request {
            SlmRequest::Listings => SlmReply::Listings(self.listings.clone()),
            SlmRequest::Listing { id } => match self.listings.iter().find(|l| l.id == *id) {
                Some(l) => SlmReply::Listings(vec![l.clone()]),
                None => SlmReply::NotFound,
            },
            SlmRequest::Create {
                folder, version, count, ..
            } => {
                let l = Listing {
                    id: self.next,
                    is_listed: false,
                    edit_url: edit_url(self.next),
                    folder: *folder,
                    version: *version,
                    count: *count,
                };
                self.next += 1;
                self.listings.push(l.clone());
                SlmReply::Listings(vec![l])
            }
            SlmRequest::Update {
                id,
                version,
                is_listed,
                count,
                ..
            } => {
                let Some(l) = self.listings.iter_mut().find(|l| l.id == *id) else {
                    return not_found();
                };
                if *is_listed && (l.folder == DRAFT || version.is_nil()) {
                    return SlmReply::Failed(SlmError::Unprocessable);
                }
                l.version = *version;
                l.count = *count;
                l.is_listed = *is_listed && *count != 0;
                SlmReply::Listings(vec![l.clone()])
            }
            SlmRequest::Associate { folder, id, version, .. } => {
                let Some(l) = self.listings.iter_mut().find(|l| l.id == *id) else {
                    return not_found();
                };
                l.folder = *folder;
                l.version = *version;
                SlmReply::Listings(vec![l.clone()])
            }
            SlmRequest::Delete { id } => {
                let Some(at) = self.listings.iter().position(|l| l.id == *id) else {
                    return not_found();
                };
                SlmReply::Listings(vec![self.listings.remove(at)])
            }
        }
    }
}

/// The sample listings: a listed hoodie with two versions, an unlisted
/// poster sold from a stock folder, a lantern folder never associated (its
/// loose items are what the audit reports), a draft SLM refuses to list,
/// and a "Products" folder of the inventory to drag from.
pub fn seed(inv: &mut Inventory, agent: Uuid) {
    if inv.root.is_nil() {
        return;
    }
    let root = super::root(inv).unwrap_or(DEMO_ROOT);
    let tr = |key: &str| -> String {
        match key {
            "hoodie" => aurora_i18n::tr!("marketplace-demo-hoodie").into(),
            "poster" => aurora_i18n::tr!("marketplace-demo-poster").into(),
            "lantern" => aurora_i18n::tr!("marketplace-demo-lantern").into(),
            "draft" => aurora_i18n::tr!("marketplace-demo-draft").into(),
            _ => aurora_i18n::tr!("marketplace-demo-products").into(),
        }
    };
    let folder = |id: Uuid, parent: Uuid, name: String, kind: i32| InvFolder {
        id,
        parent,
        name,
        type_default: kind,
        version: 1,
        ..Default::default()
    };
    let mut folders = Vec::new();
    if root == DEMO_ROOT {
        folders.push(folder(
            root,
            inv.root,
            aurora_i18n::tr!("common-marketplace-listings").into(),
            LISTINGS,
        ));
    }
    folders.extend([
        folder(HOODIE, root, tr("hoodie"), -1),
        folder(HOODIE_V1, HOODIE, format!("{} v1", tr("hoodie")), -1),
        folder(HOODIE_V2, HOODIE, format!("{} v2", tr("hoodie")), -1),
        folder(POSTER, root, tr("poster"), -1),
        folder(POSTER_VERSION, POSTER, tr("poster"), -1),
        folder(POSTER_STOCK, POSTER_VERSION, tr("poster"), STOCK),
        folder(LANTERN, root, tr("lantern"), -1),
        folder(DRAFT, root, tr("draft"), -1),
        folder(DRAFT_VERSION, DRAFT, tr("draft"), -1),
        folder(PRODUCTS, inv.root, tr("products"), -1),
    ]);
    inv.created_folders(folders);
    let sell = COPY | MODIFY | TRANSFER;
    let item = |n: u128, parent: Uuid, name: String, asset_type: i32, inv_type: i32, copy: bool| InvItem {
        id: Uuid::from_u128(8_400_100 + n),
        parent,
        name,
        asset_type,
        inv_type,
        asset_id: Uuid::from_u128(8_400_500 + n),
        creator: agent,
        owner: agent,
        base_mask: 0x7fffffff,
        owner_mask: if copy { 0x7fffffff } else { 0x7fffffff & !COPY },
        next_owner_mask: if copy { sell } else { MODIFY | TRANSFER },
        ..Default::default()
    };
    let hoodie = tr("hoodie");
    let poster = tr("poster");
    let lantern = tr("lantern");
    let draft = tr("draft");
    inv.add_items(vec![
        item(1, HOODIE_V1, format!("{hoodie} v1"), 6, 6, true),
        item(2, HOODIE_V2, format!("{hoodie} v2"), 6, 6, true),
        item(3, HOODIE_V2, aurora_i18n::tr!("marketplace-demo-instructions").into(), 7, 7, true),
        item(4, POSTER_STOCK, format!("{poster} #1"), 6, 6, false),
        item(5, POSTER_STOCK, format!("{poster} #2"), 6, 6, false),
        item(6, POSTER_STOCK, format!("{poster} #3"), 6, 6, false),
        item(7, LANTERN, lantern, 6, 6, true),
        item(8, LANTERN, aurora_i18n::tr!("marketplace-demo-lantern-no-copy").into(), 6, 6, false),
        item(9, DRAFT_VERSION, draft, 6, 6, true),
        item(10, PRODUCTS, aurora_i18n::tr!("marketplace-demo-chair").into(), 6, 6, true),
        item(11, PRODUCTS, aurora_i18n::tr!("marketplace-demo-vase").into(), 6, 6, false),
        item(12, PRODUCTS, aurora_i18n::tr!("marketplace-demo-texture").into(), 0, 0, true),
    ]);
}
