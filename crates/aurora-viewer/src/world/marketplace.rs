//! The merchant's Marketplace listings: LLMarketplaceData (Firestorm
//! indra/newview/llmarketplacefunctions.cpp) and the marketplace helpers of
//! llinventoryfunctions.cpp (depth_nesting_in_marketplace, nested_parent_id,
//! compute_stock_count, update_marketplace_category), originally LGPL 2.1.
//!
//! The records come from the SLM API (`aurora_net::slm`); the requests wait in
//! an outbox the application sends, so no HTTP ever runs on the UI thread.

use super::inventory::actions::parent;
use super::inventory::{FetchState, Inventory};
use aurora_net::slm::{Listing, MerchantStatus, SlmError, SlmReply, SlmRequest};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub mod demo;
#[cfg(test)]
mod tests;
pub mod validate;

/// FT_MARKETPLACE_LISTINGS, FT_MARKETPLACE_STOCK.
pub const LISTINGS: i32 = 53;
pub const STOCK: i32 = 54;
/// compute_stock_count: no stock folder below (unlimited), or a stock folder
/// that is not loaded yet.
pub const STOCK_INFINITE: i32 = -1;
pub const STOCK_NOT_EVALUATED: i32 = -2;
/// updateSLMListingCoro: FORCE_UPDATE_TIMEOUT before reading back a listing
/// whose update came back empty.
const FORCE_UPDATE: Duration = Duration::from_secs(5);

/// The "Marketplace listings" folder of the inventory, if there is one.
pub fn root(inv: &Inventory) -> Option<Uuid> {
    super::appearance::system_folder(inv, LISTINGS)
}

/// depth_nesting_in_marketplace: 0 for the root, 1 for a listing folder, 2
/// for a version folder…; -1 outside the listings (or without a root).
pub fn depth(inv: &Inventory, root: Uuid, id: Uuid) -> i32 {
    if root.is_nil() {
        return -1;
    }
    let mut current = id;
    for depth in 0..=inv.folders.len() as i32 + 1 {
        if current == root {
            return depth;
        }
        match parent(inv, current) {
            Some(p) if !p.is_nil() => current = p,
            _ => return -1,
        }
    }
    -1
}

/// nested_parent_id: the listing folder an object at `depth` is in (nil
/// outside the listings, or for an item right under the root).
pub fn nested_parent(inv: &Inventory, id: Uuid, depth: i32) -> Uuid {
    if depth < 1 {
        return Uuid::nil();
    }
    if depth == 1 {
        return if inv.folders.contains_key(&id) { id } else { Uuid::nil() };
    }
    let mut current = id;
    for _ in 1..depth {
        current = parent(inv, current).unwrap_or_default();
    }
    current
}

/// Whether `id` is `ancestor` or below it (isObjectDescendentOf).
fn below(inv: &Inventory, mut id: Uuid, ancestor: Uuid) -> bool {
    for _ in 0..=inv.folders.len() + 1 {
        if id == ancestor {
            return true;
        }
        match parent(inv, id) {
            Some(p) if !p.is_nil() => id = p,
            _ => return false,
        }
    }
    false
}

/// A folder the inventory knows to be outside the listings: its parents lead
/// to the inventory root without crossing the listings root. A broken chain
/// (a parent not loaded) is not taken as proof, as no listing must be removed
/// on a guess.
fn outside(inv: &Inventory, root: Uuid, mut id: Uuid) -> bool {
    for _ in 0..=inv.folders.len() + 1 {
        if id == root {
            return false;
        }
        if id == inv.root {
            return true;
        }
        match parent(inv, id) {
            Some(p) if !p.is_nil() => id = p,
            _ => return false,
        }
    }
    false
}

fn loaded(inv: &Inventory, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched)
}

/// count_descendants_items.
pub fn count_items(inv: &Inventory, id: Uuid) -> usize {
    fn walk(inv: &Inventory, id: Uuid, seen: &mut HashSet<Uuid>) -> usize {
        let Some(f) = inv.folders.get(&id) else { return 0 };
        if !seen.insert(id) {
            return 0;
        }
        f.items.len() + f.children.iter().map(|c| walk(inv, *c, seen)).sum::<usize>()
    }
    walk(inv, id, &mut HashSet::new())
}

/// The subfolders of a folder, in the inventory's order.
pub fn subfolders(inv: &Inventory, id: Uuid) -> &[Uuid] {
    inv.folders.get(&id).map_or(&[], |f| f.children.as_slice())
}

/// One record of mMarketplaceItems (LLMarketplaceTuple).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: i32,
    pub version: Uuid,
    pub listed: bool,
    pub edit_url: String,
    pub count: i32,
}

/// MarketplaceFetchCodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fetch {
    #[default]
    NotDone,
    Loading,
    Done,
    Failed,
}

/// MarketplaceStatusCodes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    NotInitialized,
    Initializing,
    Known(MerchantStatus),
}

/// The alerts LLMarketplaceData raises, shown by the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// MerchantTransactionFailed or MerchantUnprocessableEntity.
    Failed(SlmError),
    /// AlertMerchantListingNotUpdated: SLM kept other values than asked.
    NotUpdated { url: String },
    /// AlertMerchantListingActivateRequired.
    ActivateRequired,
    /// AlertMerchantVersionFolderEmpty.
    VersionEmpty,
}

#[derive(Default)]
pub struct MarketplaceData {
    status: Status,
    fetch: Fetch,
    /// mMarketplaceItems, by listing folder.
    records: HashMap<Uuid, Record>,
    /// mVersionFolders: version folder -> listing folder.
    versions: HashMap<Uuid, Uuid>,
    /// mPendingUpdateSet.
    pending: HashSet<Uuid>,
    /// The listings root while GET /listings runs.
    loading_root: Uuid,
    alerts: Vec<Alert>,
    outbox: Vec<SlmRequest>,
    /// Listings read back after an empty update answer.
    delayed: Vec<(Instant, i32)>,
}

impl MarketplaceData {
    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn fetch(&self) -> Fetch {
        self.fetch
    }

    pub fn is_merchant(&self) -> bool {
        self.status == Status::Known(MerchantStatus::Merchant)
    }

    /// initializeSLM: true when GET /merchant must be sent now.
    pub fn initialize(&mut self) -> bool {
        if self.status != Status::NotInitialized {
            return false;
        }
        self.status = Status::Initializing;
        true
    }

    pub fn set_status(&mut self, status: MerchantStatus) {
        self.status = Status::Known(status);
    }

    /// LLFloaterMarketplaceListings::fetchContents: GET /listings once.
    pub fn fetch_contents(&mut self, root: Uuid) -> bool {
        if root.is_nil() || matches!(self.fetch, Fetch::Loading | Fetch::Done) {
            return false;
        }
        self.fetch = Fetch::Loading;
        self.loading_root = root;
        self.pending.insert(root);
        self.outbox.push(SlmRequest::Listings);
        true
    }

    /// The requests to send, with the delayed read-backs that are due.
    pub fn take_requests(&mut self, now: Instant) -> Vec<SlmRequest> {
        let mut due = Vec::new();
        self.delayed.retain(|(at, id)| {
            if *at <= now {
                due.push(*id);
                false
            } else {
                true
            }
        });
        for id in due {
            self.get_slm_listing(id);
        }
        std::mem::take(&mut self.outbox)
    }

    pub fn take_alerts(&mut self) -> Vec<Alert> {
        std::mem::take(&mut self.alerts)
    }

    // Accessors

    pub fn record(&self, folder: Uuid) -> Option<&Record> {
        self.records.get(&folder)
    }

    /// isListed: the folder has a listing on the Marketplace (whether it is
    /// listed for sale or not).
    pub fn is_listed(&self, folder: Uuid) -> bool {
        self.records.contains_key(&folder)
    }

    /// getActivationState: the listing of this listing or version folder is
    /// listed for sale.
    pub fn activation_state(&self, folder: Uuid) -> bool {
        if let Some(r) = self.records.get(&folder) {
            return r.listed;
        }
        self.versions
            .get(&folder)
            .and_then(|listing| self.records.get(listing))
            .is_some_and(|r| r.listed)
    }

    pub fn listing_id(&self, folder: Uuid) -> i32 {
        self.records.get(&folder).map_or(0, |r| r.id)
    }

    pub fn count_on_hand(&self, folder: Uuid) -> i32 {
        self.records.get(&folder).map_or(-1, |r| r.count)
    }

    pub fn version_folder(&self, folder: Uuid) -> Uuid {
        self.records.get(&folder).map_or(Uuid::nil(), |r| r.version)
    }

    /// getListingFolder: the folder of a listing id.
    pub fn listing_folder(&self, id: i32) -> Uuid {
        self.records
            .iter()
            .find(|(_, r)| r.id == id)
            .map_or(Uuid::nil(), |(folder, _)| *folder)
    }

    pub fn is_version_folder(&self, folder: Uuid) -> bool {
        self.versions.contains_key(&folder)
    }

    /// getListingURL.
    pub fn listing_url(&self, inv: &Inventory, root: Uuid, folder: Uuid) -> String {
        let listing = nested_parent(inv, folder, depth(inv, root, folder));
        self.records.get(&listing).map(|r| r.edit_url.clone()).unwrap_or_default()
    }

    /// isUpdating: only listing and version folders show it.
    pub fn is_updating(&self, inv: &Inventory, root: Uuid, folder: Uuid) -> bool {
        let depth = depth(inv, root, folder);
        if depth < 1 {
            return false;
        }
        self.pending.contains(&root) || self.pending.contains(&nested_parent(inv, folder, depth))
    }

    fn set_updating(&mut self, folder: Uuid, updating: bool) {
        if updating {
            self.pending.insert(folder);
        } else {
            self.pending.remove(&folder);
        }
    }

    /// getVersionFolderIfUnique: the only subfolder, else nil and
    /// AlertMerchantListingActivateRequired.
    fn version_if_unique(&mut self, inv: &Inventory, folder: Uuid) -> Uuid {
        match subfolders(inv, folder) {
            [only] => *only,
            _ => {
                self.alerts.push(Alert::ActivateRequired);
                Uuid::nil()
            }
        }
    }

    /// compute_stock_count.
    pub fn stock_count(&self, inv: &Inventory, root: Uuid, folder: Uuid, force: bool) -> i32 {
        self.stock_count_at(inv, root, folder, force, 0)
    }

    fn stock_count_at(&self, inv: &Inventory, root: Uuid, folder: Uuid, force: bool, level: usize) -> i32 {
        let Some(f) = inv.folders.get(&folder) else {
            return STOCK_INFINITE;
        };
        if f.info.type_default == STOCK {
            if f.state != FetchState::Fetched {
                return STOCK_NOT_EVALUATED;
            }
            return f.items.len() as i32;
        }
        if !force {
            let depth = depth(inv, root, folder);
            let listing = nested_parent(inv, folder, depth);
            if !self.is_listed(listing) {
                return STOCK_INFINITE;
            }
            let version = self.version_folder(listing);
            if depth == 1 {
                return if version.is_nil() {
                    STOCK_INFINITE
                } else {
                    self.stock_count_at(inv, root, version, true, level + 1)
                };
            }
            if depth == 2 && !version.is_nil() && version != folder {
                return STOCK_INFINITE;
            }
        }
        // listings are at most a few levels deep; a cycle stops here
        if level > 16 {
            return STOCK_INFINITE;
        }
        let mut current = STOCK_INFINITE;
        for child in &f.children {
            let count = self.stock_count_at(inv, root, *child, true, level + 1);
            if current == STOCK_INFINITE || (count != STOCK_INFINITE && count < current) {
                current = count;
            }
        }
        current
    }

    // Public changes (they queue SLM requests)

    /// createListing.
    pub fn create_listing(&mut self, inv: &Inventory, root: Uuid, folder: Uuid) -> bool {
        if self.is_listed(folder) {
            return false;
        }
        let version = self.version_if_unique(inv, folder);
        let mut count = STOCK_INFINITE;
        if !version.is_nil() {
            count = self.stock_count(inv, root, version, true);
        }
        if count == STOCK_NOT_EVALUATED {
            count = 0;
        }
        let name = inv.folders.get(&folder).map(|f| f.info.name.clone()).unwrap_or_default();
        self.send(SlmRequest::Create {
            folder,
            name,
            version,
            count,
        });
        true
    }

    /// clearListing: DELETE the listing (SLM archives it).
    pub fn clear_listing(&mut self, inv: &Inventory, root: Uuid, folder: Uuid) -> bool {
        if folder.is_nil() {
            return false;
        }
        let listing = if self.is_listed(folder) {
            folder
        } else {
            nested_parent(inv, folder, depth(inv, root, folder))
        };
        let id = self.listing_id(listing);
        if id == 0 {
            return false;
        }
        self.set_updating(listing, true);
        self.outbox.push(SlmRequest::Delete { id });
        true
    }

    fn get_slm_listing(&mut self, id: i32) {
        let folder = self.listing_folder(id);
        self.set_updating(folder, true);
        self.outbox.push(SlmRequest::Listing { id });
    }

    pub fn refresh_listing(&mut self, inv: &Inventory, root: Uuid, folder: Uuid) {
        let listing = nested_parent(inv, folder, depth(inv, root, folder));
        let id = self.listing_id(listing);
        if id > 0 && !self.is_updating(inv, root, folder) {
            self.get_slm_listing(id);
        }
    }

    /// activateListing: list or unlist.
    pub fn activate_listing(&mut self, inv: &Inventory, root: Uuid, folder: Uuid, activate: bool) -> bool {
        let listing = nested_parent(inv, folder, depth(inv, root, folder));
        let id = self.listing_id(listing);
        if id == 0 {
            return false;
        }
        if self.activation_state(listing) == activate {
            return true;
        }
        let version = self.version_folder(listing);
        let mut count = self.stock_count(inv, root, folder, false);
        if count == STOCK_NOT_EVALUATED {
            // a local problem must not change what SLM has
            count = self.count_on_hand(listing);
        }
        self.send(SlmRequest::Update {
            folder: listing,
            id,
            version,
            is_listed: activate,
            count,
        });
        true
    }

    /// setVersionFolder: a nil version clears it, and unlists.
    pub fn set_version_folder(&mut self, inv: &Inventory, root: Uuid, folder: Uuid, version: Uuid) -> bool {
        let listing = nested_parent(inv, folder, depth(inv, root, folder));
        let id = self.listing_id(listing);
        if id == 0 {
            return false;
        }
        if self.version_folder(listing) == version {
            return true;
        }
        let is_listed = !version.is_nil() && self.activation_state(listing);
        let mut count = self.stock_count(inv, root, version, false);
        if count == STOCK_NOT_EVALUATED {
            count = 0;
        }
        self.send(SlmRequest::Update {
            folder: listing,
            id,
            version,
            is_listed,
            count,
        });
        true
    }

    /// updateCountOnHand: the local count is kept at once, so a burst of
    /// restocking sends one update per change only.
    pub fn update_count_on_hand(&mut self, inv: &Inventory, root: Uuid, folder: Uuid) -> bool {
        let listing = nested_parent(inv, folder, depth(inv, root, folder));
        let id = self.listing_id(listing);
        if id == 0 {
            return false;
        }
        let count = self.stock_count(inv, root, folder, false);
        if count == self.count_on_hand(listing) {
            return true;
        }
        if count == STOCK_NOT_EVALUATED {
            return false;
        }
        let is_listed = self.activation_state(listing);
        let version = self.version_folder(listing);
        self.send(SlmRequest::Update {
            folder: listing,
            id,
            version,
            is_listed,
            count,
        });
        if let Some(r) = self.records.get_mut(&listing) {
            r.count = count;
        }
        true
    }

    /// associateListing: an existing listing (by its id) on this folder.
    pub fn associate_listing(&mut self, inv: &Inventory, folder: Uuid, source: Uuid, id: i32) -> bool {
        if self.is_listed(folder) {
            return false;
        }
        let version = self.version_if_unique(inv, folder);
        self.send(SlmRequest::Associate {
            folder,
            id,
            version,
            source,
        });
        true
    }

    fn send(&mut self, request: SlmRequest) {
        for folder in request.folders() {
            self.set_updating(folder, true);
        }
        self.outbox.push(request);
    }

    // The answers (the SLM coroutines after their request)

    /// The end of a request: records, pending folders and alerts.
    pub fn apply(&mut self, inv: &Inventory, root: Uuid, request: &SlmRequest, reply: SlmReply, now: Instant) {
        if let SlmRequest::Listings = request {
            let loading = std::mem::take(&mut self.loading_root);
            self.set_updating(loading, false);
        }
        for folder in request.folders() {
            self.set_updating(folder, false);
        }
        if let SlmRequest::Listing { id } | SlmRequest::Delete { id } = request {
            let folder = self.listing_folder(*id);
            self.set_updating(folder, false);
        }
        let listings = match reply {
            SlmReply::Listings(listings) => listings,
            SlmReply::NotFound => {
                if let SlmRequest::Listing { id } = request {
                    let folder = self.listing_folder(*id);
                    self.delete_listing(folder);
                }
                return;
            }
            SlmReply::Failed(error) => {
                log::warn!("SLM {} failed", request.name());
                if let SlmRequest::Listings = request {
                    self.fetch = Fetch::Failed;
                }
                self.alerts.push(Alert::Failed(error));
                return;
            }
        };
        match request {
            SlmRequest::Listings => {
                for l in listings {
                    self.add_listing(l);
                }
                self.fetch = Fetch::Done;
            }
            SlmRequest::Listing { .. } => {
                for l in listings {
                    self.update_record(&l);
                }
            }
            SlmRequest::Create { folder, .. } => {
                if listings.is_empty() {
                    log::info!("SLM: empty answer to the creation of a listing for folder {folder}");
                }
                for l in listings {
                    self.add_listing(l);
                }
            }
            SlmRequest::Update {
                id, version, is_listed, ..
            } => {
                if listings.is_empty() {
                    // read the listing back a little later
                    self.delayed.push((now + FORCE_UPDATE, *id));
                    return;
                }
                for l in listings {
                    self.update_record(&l);
                    if *is_listed != l.is_listed || *version != l.version {
                        self.alerts.push(Alert::NotUpdated { url: l.edit_url.clone() });
                    }
                }
            }
            SlmRequest::Associate { .. } => {
                for l in listings {
                    let old = self.listing_folder(l.id);
                    if !old.is_nil() {
                        self.delete_listing(old);
                    }
                    let folder = l.folder;
                    self.add_listing(l);
                    // the stock count follows the new local folder
                    self.update_count_on_hand(inv, root, folder);
                }
            }
            SlmRequest::Delete { .. } => {
                for l in listings {
                    let folder = self.listing_folder(l.id);
                    self.delete_listing(folder);
                }
            }
        }
    }

    /// addListing.
    fn add_listing(&mut self, l: Listing) {
        if let Some(old) = self.records.get(&l.folder)
            && self.versions.get(&old.version) == Some(&l.folder)
        {
            self.versions.remove(&old.version);
        }
        if !l.version.is_nil() {
            self.versions.insert(l.version, l.folder);
        }
        self.records.insert(
            l.folder,
            Record {
                id: l.id,
                version: l.version,
                listed: l.is_listed,
                edit_url: l.edit_url,
                count: l.count,
            },
        );
    }

    /// setListingID, setVersionFolderID, setActivationState, setListingURL
    /// and setCountOnHand of an existing record.
    fn update_record(&mut self, l: &Listing) {
        let Some(r) = self.records.get_mut(&l.folder) else { return };
        r.id = l.id;
        r.listed = l.is_listed;
        r.edit_url.clone_from(&l.edit_url);
        r.count = l.count;
        if r.version != l.version {
            let old = std::mem::replace(&mut r.version, l.version);
            self.versions.remove(&old);
            if !l.version.is_nil() {
                self.versions.insert(l.version, l.folder);
            }
        }
    }

    /// deleteListing.
    fn delete_listing(&mut self, folder: Uuid) -> bool {
        let Some(r) = self.records.remove(&folder) else {
            return false;
        };
        self.versions.remove(&r.version);
        true
    }

    /// update_marketplace_category's consistency checks, for every listing,
    /// after the inventory changed: a listing folder moved out of the
    /// listings is disassociated, a version folder moved away is cleared, an
    /// empty listed version is unlisted, and the stock follows the inventory.
    pub fn reconcile(&mut self, inv: &Inventory, root: Uuid) {
        if !self.is_merchant() || self.fetch != Fetch::Done || root.is_nil() || self.pending.contains(&root) {
            return;
        }
        let mut folders: Vec<Uuid> = self.records.keys().copied().collect();
        folders.sort();
        for listing in folders {
            if self.pending.contains(&listing) || !inv.folders.contains_key(&listing) {
                continue;
            }
            let depth = depth(inv, root, listing);
            if depth < 0 {
                if outside(inv, root, listing) {
                    log::info!("SLM: disassociate a listing whose folder left the Marketplace listings");
                    self.clear_listing(inv, root, listing);
                }
                continue;
            }
            if depth != 1 || !loaded(inv, listing) {
                continue;
            }
            let version = self.version_folder(listing);
            if !version.is_nil() && (!below(inv, version, listing) || self::depth(inv, root, version) != 2) {
                log::info!("SLM: clear a version folder that is no longer in its listing");
                self.set_version_folder(inv, root, listing, Uuid::nil());
                continue;
            }
            if !version.is_nil() && loaded(inv, version) && self.activation_state(version) && count_items(inv, version) == 0 {
                log::info!("SLM: unlist a listing whose version folder is empty");
                self.alerts.push(Alert::VersionEmpty);
                self.activate_listing(inv, root, listing, false);
                continue;
            }
            if self.stock_count(inv, root, listing, false) != self.count_on_hand(listing) {
                self.update_count_on_hand(inv, root, listing);
            }
        }
    }
}
