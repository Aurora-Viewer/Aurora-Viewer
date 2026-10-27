//! LLMarketplaceValidator: validate_marketplacelistings and the checks it
//! shares with dragging into the listings (can_move_to_marketplace,
//! can_move_item_to_marketplace, can_move_folder_to_marketplace,
//! has_correct_permissions_for_sale, move_*_to_marketplacelistings) of
//! Firestorm indra/newview/llinventoryfunctions.cpp, originally LGPL 2.1.
//!
//! Firestorm fixes a hierarchy through asynchronous folder creations. Here a
//! working copy of the folders (`Tree`) takes each fix at once, so the rest of
//! the audit sees the fixed hierarchy, and the fixes become one inventory
//! `Mutation` sent afterwards.

use super::STOCK;
use crate::world::inventory::Inventory;
use crate::world::inventory::actions::{COPY, TRANSFER};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// InventoryOutboxMaxFolderCount, …MaxFolderDepth, …MaxItemCount and
/// …MaxStockItemCount (settings.xml).
pub const MAX_FOLDER_COUNT: usize = 20;
pub const MAX_FOLDER_DEPTH: i32 = 4;
pub const MAX_ITEM_COUNT: usize = 200;
pub const MAX_STOCK_ITEM_COUNT: usize = 200;

/// The key of copyable items (LLInventoryType::IT_COUNT in Firestorm): above
/// every real inventory type, so they sort last like there.
const COPYABLE_KEY: u32 = 0xFF << 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// One line of the audit (the cb_msg callback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The listing depth the line was written at (1 = a listing folder).
    pub depth: i32,
    /// Indentation steps (Firestorm's four spaces).
    pub indent: usize,
    pub level: Level,
    pub text: String,
}

/// What an audit found, and the fixes it made when asked to.
#[derive(Debug, Default)]
pub struct Report {
    pub messages: Vec<Message>,
    pub ok: bool,
    /// AlertMerchantStockFolderSplit: a stock folder was split.
    pub split: bool,
    /// The inventory changes of the fixes (empty without fixes).
    pub change: Mutation,
}

impl Report {
    /// LLFloaterMarketplaceValidation::handleCurrentListing: only the
    /// listings with a warning or an error are kept.
    pub fn shown(&self) -> Vec<Message> {
        let mut out = Vec::new();
        let mut current: Vec<Message> = Vec::new();
        let flush = |current: &mut Vec<Message>, out: &mut Vec<Message>| {
            if current.iter().any(|m| m.level > Level::Info) {
                out.append(current);
            }
            current.clear();
        };
        for m in &self.messages {
            if m.depth == 1 {
                flush(&mut current, &mut out);
            }
            current.push(m.clone());
        }
        flush(&mut current, &mut out);
        out
    }

    /// The text of MerchantListingFailed / MerchantFolderActivationFailed
    /// (LLFolderBridge::gatherMessage keeps the errors).
    pub fn errors(&self) -> String {
        self.messages
            .iter()
            .filter(|m| m.level == Level::Error)
            .map(|m| m.text.trim().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// can_move_to_marketplace: why an item cannot be sold, if it cannot.
pub fn sellable(inv: &Inventory, item: Uuid, worn: &HashSet<Uuid>) -> Result<(), String> {
    let Some(it) = inv.items.get(&item) else {
        return Err(aurora_i18n::tr!("common-item-not-found").into());
    };
    if matches!(it.asset_type, 24 | 25) {
        return Err(aurora_i18n::tr!("marketplace-reason-linked").into());
    }
    if it.owner_mask & TRANSFER == 0 {
        return Err(aurora_i18n::tr!("marketplace-reason-no-transfer").into());
    }
    if worn.contains(&item) {
        return Err(aurora_i18n::tr!("marketplace-reason-worn").into());
    }
    if !crate::world::inventory::actions::under(inv, item, inv.root) || inv.root.is_nil() {
        return Err(aurora_i18n::tr!("marketplace-reason-not-in-inventory").into());
    }
    if it.asset_type == 2 {
        return Err(aurora_i18n::tr!("marketplace-reason-calling-card").into());
    }
    Ok(())
}

fn copyable(inv: &Inventory, item: Uuid) -> bool {
    inv.items.get(&item).is_some_and(|it| it.owner_mask & COPY != 0)
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    kind: i32,
    parent: Uuid,
    children: Vec<Uuid>,
    items: Vec<Uuid>,
}

/// A working copy of the folders an audit or a drop touches, read from the
/// inventory as it goes, with the changes made so far.
pub struct Tree<'a> {
    inv: &'a Inventory,
    worn: &'a HashSet<Uuid>,
    root: Uuid,
    nodes: HashMap<Uuid, Node>,
    item_parent: HashMap<Uuid, Uuid>,
    copies: HashMap<Uuid, Uuid>,
    created: HashSet<Uuid>,
    change: Mutation,
}

impl<'a> Tree<'a> {
    pub fn new(inv: &'a Inventory, root: Uuid, worn: &'a HashSet<Uuid>) -> Self {
        Tree {
            inv,
            worn,
            root,
            nodes: HashMap::new(),
            item_parent: HashMap::new(),
            copies: HashMap::new(),
            created: HashSet::new(),
            change: Mutation::default(),
        }
    }

    fn node(&mut self, id: Uuid) -> Option<&Node> {
        if !self.nodes.contains_key(&id) {
            let f = self.inv.folders.get(&id)?;
            self.nodes.insert(
                id,
                Node {
                    name: f.info.name.clone(),
                    kind: f.info.type_default,
                    parent: f.info.parent,
                    children: f.children.clone(),
                    items: f.items.clone(),
                },
            );
        }
        self.nodes.get(&id)
    }

    fn source(&self, id: Uuid) -> Uuid {
        self.copies.get(&id).copied().unwrap_or(id)
    }

    fn item(&self, id: Uuid) -> Option<&aurora_net::inventory::InvItem> {
        self.inv.items.get(&self.source(id))
    }

    fn copyable(&self, id: Uuid) -> bool {
        copyable(self.inv, self.source(id))
    }

    fn sellable(&self, id: Uuid) -> Result<(), String> {
        sellable(self.inv, self.source(id), self.worn)
    }

    fn copy_item(&mut self, item: Uuid, to: Uuid) -> Result<(), String> {
        if !self.copyable(item) {
            return Err(aurora_i18n::tr!("world-inventory-actions-the-selection-contains-a-no-copy-item").into());
        }
        let id = Uuid::new_v4();
        self.copies.insert(id, self.source(item));
        self.node(to);
        if let Some(node) = self.nodes.get_mut(&to) {
            node.items.push(id);
        }
        self.item_parent.insert(id, to);
        self.change.refresh.push(to);
        Ok(())
    }

    fn copy_folder(&mut self, folder: Uuid, to: Uuid, seen: &mut HashSet<Uuid>) -> Result<Uuid, String> {
        if !seen.insert(folder)
            || self
                .inv
                .folders
                .get(&folder)
                .is_none_or(|f| f.state != crate::world::inventory::FetchState::Fetched)
        {
            return Err(aurora_i18n::tr!("world-inventory-actions-load-all-subfolders-before-copying-this-folder").into());
        }
        let name = self.name(folder);
        let kind = self.kind(folder);
        let copy = self.create(to, kind, &name);
        // These are source categories to audit, rather than wrappers the
        // audit itself just produced (which its recursive walk skips).
        self.created.remove(&copy);
        for child in self.children(folder) {
            self.copy_folder(child, copy, seen)?;
        }
        for item in self.items(folder) {
            // copy_inventory_category with move_no_copy_items=false leaves
            // stock in the original folder when copying a category.
            if self.copyable(item) {
                self.copy_item(item, copy)?;
            }
        }
        Ok(copy)
    }

    fn is_folder(&mut self, id: Uuid) -> bool {
        self.node(id).is_some()
    }

    fn name(&mut self, id: Uuid) -> String {
        if let Some(n) = self.node(id) {
            return n.name.clone();
        }
        self.item(id).map(|it| it.name.clone()).unwrap_or_default()
    }

    fn kind(&mut self, id: Uuid) -> i32 {
        self.node(id).map_or(-1, |n| n.kind)
    }

    fn children(&mut self, id: Uuid) -> Vec<Uuid> {
        self.node(id).map(|n| n.children.clone()).unwrap_or_default()
    }

    fn items(&mut self, id: Uuid) -> Vec<Uuid> {
        self.node(id).map(|n| n.items.clone()).unwrap_or_default()
    }

    fn parent(&mut self, id: Uuid) -> Uuid {
        if let Some(n) = self.node(id) {
            return n.parent;
        }
        self.item_parent
            .get(&id)
            .copied()
            .or_else(|| self.item(id).map(|it| it.parent))
            .unwrap_or_default()
    }

    /// Whether `id` is `ancestor` or below it, as the tree is now.
    fn below(&mut self, mut id: Uuid, ancestor: Uuid) -> bool {
        for _ in 0..64 {
            if id == ancestor {
                return true;
            }
            let p = self.parent(id);
            if p.is_nil() {
                return false;
            }
            id = p;
        }
        false
    }

    /// depth_nesting_in_marketplace on the tree.
    pub fn depth(&mut self, mut id: Uuid) -> i32 {
        if self.root.is_nil() {
            return -1;
        }
        for depth in 0..64 {
            if id == self.root {
                return depth;
            }
            let p = self.parent(id);
            if p.is_nil() {
                return -1;
            }
            id = p;
        }
        -1
    }

    /// get_folder_levels.
    fn levels(&mut self, id: Uuid, guard: usize) -> i32 {
        if guard > 64 {
            return 1;
        }
        let children = self.children(id);
        1 + children.into_iter().map(|c| self.levels(c, guard + 1)).max().unwrap_or(0)
    }

    /// get_folder_path_length.
    fn path_length(&mut self, ancestor: Uuid, descendant: Uuid) -> i32 {
        if ancestor == descendant {
            return 0;
        }
        let mut id = descendant;
        for depth in 1..64 {
            if !self.is_folder(id) {
                break;
            }
            let p = self.parent(id);
            if p.is_nil() {
                break;
            }
            if p == ancestor {
                return depth;
            }
            id = p;
        }
        -1
    }

    /// getFirstDescendantOf: the child of `master` that holds `id`.
    fn first_descendant_of(&mut self, master: Uuid, mut id: Uuid) -> Option<Uuid> {
        for _ in 0..64 {
            let p = self.parent(id);
            if p.is_nil() {
                return None;
            }
            if p == master {
                return Some(id);
            }
            id = p;
        }
        None
    }

    /// collectDescendents: (folders, items) below a folder.
    fn descendants(&mut self, id: Uuid) -> (Vec<Uuid>, Vec<Uuid>) {
        let mut folders = Vec::new();
        let mut items = Vec::new();
        let mut todo = vec![id];
        let mut seen = HashSet::new();
        while let Some(f) = todo.pop() {
            if !seen.insert(f) {
                continue;
            }
            items.extend(self.items(f));
            for c in self.children(f) {
                folders.push(c);
                todo.push(c);
            }
        }
        (folders, items)
    }

    fn counts(&mut self, folders: &[Uuid], items: &[Uuid]) -> (usize, usize) {
        let stock_folders = folders.iter().filter(|f| self.kind(**f) == STOCK).count();
        let copyables = items.iter().filter(|i| self.copyable(**i)).count();
        (copyables + stock_folders, items.len() - copyables)
    }

    /// LLViewerInventoryCategory::acceptItem: a stock folder takes no-copy
    /// items of one type and one set of next-owner permissions.
    fn accepts(&mut self, folder: Uuid, item: Uuid) -> bool {
        if self.kind(folder) != STOCK {
            return true;
        }
        let Some(it) = self.item(item) else { return false };
        if it.owner_mask & COPY != 0 {
            return false;
        }
        let (kind, next) = (it.inv_type, it.next_owner_mask);
        match self.items(folder).first().and_then(|first| self.item(*first)) {
            None => true,
            Some(first) => first.inv_type == kind && first.next_owner_mask == next,
        }
    }

    /// can_move_item_to_marketplace.
    pub fn can_move_item(&mut self, master: Option<Uuid>, dest: Uuid, item: Uuid, bundle: usize, from_paste: bool) -> Result<(), String> {
        let in_stock = self.kind(dest) == STOCK;
        if !self.accepts(dest, item) {
            return Err(aurora_i18n::tr!("marketplace-reason-mixed-stock").into());
        }
        self.sellable(item)?;
        let mut item_count = if in_stock { 0 } else { bundle };
        let mut stock_count = if in_stock { bundle } else { 0 };
        let mut folder_count = 0;
        let version = master.filter(|m| *m != dest).and_then(|m| self.first_descendant_of(m, dest));
        if let Some(version) = version {
            if !from_paste && self.below(item, version) {
                item_count = 0;
            }
            let (folders, items) = self.descendants(version);
            let (items_n, stock_n) = self.counts(&folders, &items);
            item_count += items_n;
            stock_count += stock_n;
            folder_count += folders.len();
            if !self.copyable(item) && !in_stock {
                folder_count += 1;
            }
        }
        limits(item_count, stock_count, folder_count)
    }

    /// can_move_folder_to_marketplace.
    pub fn can_move_folder(
        &mut self,
        master: Option<Uuid>,
        dest: Uuid,
        folder: Uuid,
        bundle: usize,
        check_items: bool,
        from_paste: bool,
    ) -> Result<(), String> {
        let incoming = self.levels(folder, 0);
        let insertion = master.map_or(1, |m| self.path_length(m, dest) + 1);
        let version = if insertion >= 2 {
            master.and_then(|m| self.first_descendant_of(m, dest))
        } else {
            None
        };
        if incoming + insertion - 2 > MAX_FOLDER_DEPTH {
            return Err(aurora_i18n::tr!("marketplace-reason-folder-levels", amount = MAX_FOLDER_DEPTH));
        }
        let (folders, items) = self.descendants(folder);
        let (mut dragged_items, mut dragged_stock) = self.counts(&folders, &items);
        let mut dragged_folders = folders.len() + bundle;
        let (mut folder_count, mut item_count, mut stock_count) = (0, 0, 0);
        if let Some(version) = version {
            if !from_paste && self.below(folder, version) {
                (dragged_folders, dragged_items, dragged_stock) = (0, 0, 0);
            }
            let (existing_folders, existing_items) = self.descendants(version);
            let (items_n, stock_n) = self.counts(&existing_folders, &existing_items);
            folder_count += existing_folders.len();
            item_count += items_n;
            stock_count += stock_n;
        }
        let (folders_total, items_total, stock_total) = (
            folder_count + dragged_folders,
            item_count + dragged_items,
            stock_count + dragged_stock,
        );
        if folders_total > MAX_FOLDER_COUNT {
            return Err(aurora_i18n::tr!("marketplace-reason-too-many-folders", amount = MAX_FOLDER_COUNT));
        }
        if items_total > MAX_ITEM_COUNT {
            return Err(aurora_i18n::tr!("marketplace-reason-too-many-objects", amount = MAX_ITEM_COUNT));
        }
        if stock_total > MAX_STOCK_ITEM_COUNT {
            return Err(aurora_i18n::tr!("marketplace-reason-too-many-stock", amount = MAX_STOCK_ITEM_COUNT));
        }
        if check_items {
            for item in items {
                self.sellable(item)?;
            }
        }
        Ok(())
    }

    /// has_correct_permissions_for_sale.
    pub fn permissions_for_sale(&mut self, folder: Uuid) -> Result<(), String> {
        let (_, items) = self.descendants(folder);
        for item in items {
            self.sellable(item)?;
        }
        Ok(())
    }

    // Changes

    fn create(&mut self, parent: Uuid, kind: i32, name: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.node(parent);
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.push(id);
        }
        self.nodes.insert(
            id,
            Node {
                name: name.to_owned(),
                kind,
                parent,
                children: Vec::new(),
                items: Vec::new(),
            },
        );
        self.created.insert(id);
        self.change.operations.push(Operation::Folder {
            id,
            parent,
            name: name.to_owned(),
            type_default: kind,
        });
        self.change.refresh.extend([parent, id]);
        id
    }

    pub fn move_item(&mut self, item: Uuid, to: Uuid) {
        let from = self.parent(item);
        if from == to {
            return;
        }
        self.node(from);
        self.node(to);
        if let Some(n) = self.nodes.get_mut(&from) {
            n.items.retain(|i| *i != item);
        }
        if let Some(n) = self.nodes.get_mut(&to) {
            n.items.push(item);
        }
        self.item_parent.insert(item, to);
        if !self.copies.contains_key(&item) {
            self.change.operations.push(Operation::Patch {
                id: item,
                folder: false,
                body: llsd_map! { "parent_id" => to },
            });
        }
        self.change.refresh.extend([from, to]);
    }

    pub fn move_folder(&mut self, folder: Uuid, to: Uuid) {
        let from = self.parent(folder);
        if from == to || folder == to {
            return;
        }
        self.node(from);
        self.node(to);
        if let Some(n) = self.nodes.get_mut(&from) {
            n.children.retain(|c| *c != folder);
        }
        if let Some(n) = self.nodes.get_mut(&to) {
            n.children.push(folder);
        }
        if let Some(n) = self.nodes.get_mut(&folder) {
            n.parent = to;
        }
        self.change.operations.push(Operation::Patch {
            id: folder,
            folder: true,
            body: llsd_map! { "parent_id" => to },
        });
        self.change.refresh.extend([from, to]);
    }

    /// LLInventoryModel::removeCategory: to the Trash.
    fn remove(&mut self, folder: Uuid) {
        match super::super::appearance::system_folder(self.inv, 14) {
            Some(trash) => self.move_folder(folder, trash),
            None => {
                let from = self.parent(folder);
                if let Some(n) = self.nodes.get_mut(&from) {
                    n.children.retain(|c| *c != folder);
                }
                self.change.operations.push(Operation::Delete { id: folder, folder: true });
                self.change.refresh.push(from);
            }
        }
    }

    fn is_empty(&mut self, folder: Uuid) -> bool {
        self.node(folder).is_none_or(|n| n.children.is_empty() && n.items.is_empty())
    }

    pub fn into_change(self) -> Mutation {
        let mut change = self.change;
        for (id, source) in self.copies {
            if let Some(item) = self.inv.items.get(&source) {
                change.copies.push(CopyItem {
                    item: source,
                    owner: item.owner,
                    parent: self.item_parent.get(&id).copied().unwrap_or_default(),
                    name: String::new(),
                });
            }
        }
        let mut seen = HashSet::new();
        change.refresh.retain(|id| !id.is_nil() && seen.insert(*id));
        change
    }

    // The audit

    fn say(&self, report: &mut Report, depth: i32, indent: usize, level: Level, text: String) {
        report.messages.push(Message {
            depth,
            indent,
            level,
            text,
        });
    }

    /// validate_marketplacelistings.
    fn validate(&mut self, cat: Uuid, mut fix: bool, mut depth: i32, report: &mut Report, guard: usize) {
        if guard > 32 || !self.is_folder(cat) {
            return;
        }
        let kind = self.kind(cat);
        let name = self.name(cat);
        if depth < 0 {
            depth = self.depth(cat);
        }
        if depth < 0 {
            // a folder outside the listings is audited as a listing, unchanged
            depth = 1;
            fix = false;
        }
        let indent = (depth - 1).max(0) as usize;

        if depth == 2
            && let Err(reason) = self.can_move_folder(Some(cat), cat, cat, 0, fix, false)
        {
            report.ok = false;
            let text = aurora_i18n::tr!("marketplace-check-error", name = name.clone(), reason = reason);
            self.say(report, depth, indent, Level::Error, text);
        }

        if kind == STOCK && depth <= 2 {
            if fix {
                let text = aurora_i18n::tr!("marketplace-check-warning-stock-level", name = name.clone());
                self.say(report, depth, indent, Level::Warn, text);
                // nest the stock folder one level deeper and go on from there
                let parent = self.parent(cat);
                let wrapper = self.create(parent, -1, &name);
                self.move_folder(cat, wrapper);
                self.validate(wrapper, fix, depth + 1, report, guard + 1);
                report.ok = false;
                return;
            }
            report.ok = false;
            let text = aurora_i18n::tr!("marketplace-check-error-stock-level", name = name.clone());
            self.say(report, depth, indent, Level::Error, text);
        }

        let items = self.items(cat);
        let subfolders = self.children(cat);
        let mut groups: BTreeMap<u32, Vec<Uuid>> = BTreeMap::new();
        let mut bad_items = false;
        for item in &items {
            if let Err(reason) = self.sellable(*item) {
                bad_items = true;
                if fix {
                    let text = aurora_i18n::tr!("marketplace-check-error", name = self.name(*item), reason = reason);
                    self.say(report, depth, indent, Level::Error, text);
                }
                continue;
            }
            let Some(it) = self.item(*item) else { continue };
            let key = if it.owner_mask & COPY != 0 {
                COPYABLE_KEY
            } else {
                ((it.inv_type as u32 & 0xFF) << 24) | (it.next_owner_mask & 0xFF_FFFF)
            };
            groups.entry(key).or_default().push(*item);
        }
        let count = groups.len();
        let unique = if count == 1 {
            *groups.keys().next().unwrap_or(&COPYABLE_KEY)
        } else {
            COPYABLE_KEY
        };

        let mut removed = false;
        if count == 0 && !bad_items {
            if subfolders.is_empty() {
                let text = if depth == 2 {
                    aurora_i18n::tr!("marketplace-check-empty-version", name = name.clone())
                } else if kind == STOCK && depth > 2 {
                    aurora_i18n::tr!("marketplace-check-empty-stock", name = name.clone())
                } else {
                    aurora_i18n::tr!("marketplace-check-empty", name = name.clone())
                };
                self.say(report, depth, indent, Level::Warn, text);
            } else if report.ok && depth >= 1 {
                self.say(report, depth, indent, Level::Info, name.clone());
            }
        } else if count == 1
            && !bad_items
            && ((unique == COPYABLE_KEY && depth > 1) || (kind == STOCK && depth > 2 && subfolders.is_empty()))
        {
            if report.ok && depth >= 1 {
                self.say(report, depth, indent, Level::Info, name.clone());
            }
        } else if fix && !bad_items {
            if kind == STOCK && (count >= 2 || !subfolders.is_empty()) {
                report.split = true;
            }
            if count > 1 || depth == 1 || (kind == STOCK && unique == COPYABLE_KEY) || (kind != STOCK && unique != COPYABLE_KEY) {
                for (key, ids) in groups {
                    let parent = if depth > 2 { self.parent(cat) } else { cat };
                    let new_kind = if key == COPYABLE_KEY { -1 } else { STOCK };
                    let text = if new_kind == STOCK {
                        aurora_i18n::tr!("marketplace-check-create-stock", name = name.clone())
                    } else {
                        aurora_i18n::tr!("marketplace-check-create-version", name = name.clone())
                    };
                    self.say(report, depth, indent, Level::Warn, text);
                    let folder = self.create(parent, new_kind, &name);
                    for item in ids.iter().rev() {
                        let text = aurora_i18n::tr!("marketplace-check-move", name = self.name(*item));
                        self.say(report, depth, indent, Level::Warn, text);
                        self.move_item(*item, folder);
                    }
                    if cat != parent && !removed && self.is_empty(cat) {
                        let text = aurora_i18n::tr!("marketplace-check-delete", name = name.clone());
                        self.say(report, depth, indent, Level::Warn, text);
                        self.remove(cat);
                        removed = true;
                    }
                }
            }
            if kind == STOCK && !removed {
                // a stock folder has no subfolder: they go up a level
                let parent = self.parent(cat);
                for sub in self.children(cat) {
                    self.move_folder(sub, parent);
                    self.validate(sub, fix, depth, report, guard + 1);
                }
            }
        } else {
            // report everything that is wrong, without changing anything
            if report.ok && depth >= 1 {
                if kind == STOCK && count >= 2 {
                    report.ok = false;
                    let text = aurora_i18n::tr!("marketplace-check-mixed-stock", name = name.clone());
                    self.say(report, depth, indent, Level::Error, text);
                } else if kind == STOCK && !subfolders.is_empty() {
                    report.ok = false;
                    let text = aurora_i18n::tr!("marketplace-check-subfolder-in-stock", name = name.clone());
                    self.say(report, depth, indent, Level::Error, text);
                } else {
                    self.say(report, depth, indent, Level::Info, name.clone());
                }
            }
            for item in &items {
                let item_name = self.name(*item);
                let text = if let Err(reason) = self.sellable(*item) {
                    aurora_i18n::tr!("marketplace-check-error", name = item_name, reason = reason)
                } else if !self.copyable(*item) && kind != STOCK {
                    aurora_i18n::tr!("marketplace-check-stock-item", name = item_name)
                } else if depth == 1 {
                    aurora_i18n::tr!("marketplace-check-unwrapped", name = item_name)
                } else {
                    continue;
                };
                report.ok = false;
                self.say(report, depth, indent + 1, Level::Error, text);
            }
        }
        if removed {
            report.ok &= !bad_items;
            return;
        }

        // the same audit on every subfolder, by name, but not on the
        // folders the fixes just made
        let mut subs: Vec<Uuid> = self.children(cat).into_iter().filter(|c| !self.created.contains(c)).collect();
        let mut named: Vec<(String, Uuid)> = subs.drain(..).map(|c| (self.name(c), c)).collect();
        named.sort();
        for (_, sub) in named {
            self.validate(sub, fix, depth + 1, report, guard + 1);
        }
        report.ok &= !bad_items;
    }
}

fn limits(items: usize, stock: usize, folders: usize) -> Result<(), String> {
    if items > MAX_ITEM_COUNT {
        return Err(aurora_i18n::tr!("marketplace-reason-too-many-objects", amount = MAX_ITEM_COUNT));
    }
    if stock > MAX_STOCK_ITEM_COUNT {
        return Err(aurora_i18n::tr!("marketplace-reason-too-many-stock", amount = MAX_STOCK_ITEM_COUNT));
    }
    if folders > MAX_FOLDER_COUNT {
        return Err(aurora_i18n::tr!("marketplace-reason-too-many-folders", amount = MAX_FOLDER_COUNT));
    }
    Ok(())
}

/// LLMarketplaceValidator::validateMarketplaceListings: audit `folder` (the
/// whole listings when nil) and, with `fix`, fix what can be fixed. `depth`
/// is the folder's listing depth, or -1 to compute it.
pub fn validate(inv: &Inventory, root: Uuid, folder: Uuid, fix: bool, depth: i32, worn: &HashSet<Uuid>) -> Report {
    let folder = if folder.is_nil() { root } else { folder };
    let mut tree = Tree::new(inv, root, worn);
    let mut report = Report {
        ok: true,
        ..Default::default()
    };
    tree.validate(folder, fix, depth, &mut report, 0);
    report.change = tree.into_change();
    report
}

/// LLInvFVBridge::addMarketplaceContextMenuOptions: a new child folder
/// must fit the version's folder count and depth, and cannot be stock's child.
pub fn can_create_folder(inv: &Inventory, root: Uuid, dest: Uuid) -> bool {
    let worn = HashSet::new();
    let mut tree = Tree::new(inv, root, &worn);
    let depth = tree.depth(dest);
    if !(0..=MAX_FOLDER_DEPTH + 1).contains(&depth) || tree.kind(dest) == STOCK {
        return false;
    }
    if depth >= 2 {
        let version = super::nested_parent(inv, dest, depth - 1);
        return tree.descendants(version).0.len() < MAX_FOLDER_COUNT;
    }
    true
}

/// What dragging inventory into the listings does: the checks of
/// LLFolderBridge::dragItemIntoFolder / dragCategoryIntoFolder, then
/// move_item_to_marketplacelistings (the folders an item needs) or
/// move_folder_to_marketplacelistings (the move, then an audit that fixes the
/// destination). Dragging always moves, like the Legacy Viewer.
pub fn drop_plan(inv: &Inventory, root: Uuid, dest: Uuid, ids: &[Uuid], worn: &HashSet<Uuid>) -> Result<Mutation, String> {
    paste_plan(inv, root, dest, ids, false, worn)
}

/// LLFolderBridge::pasteFromClipboard: copy or move with the same limits,
/// stock wrappers and fixing audit as a drop. Copies remain virtual until the
/// hierarchy is final, so normalization can never move an original item.
pub fn paste_plan(inv: &Inventory, root: Uuid, dest: Uuid, ids: &[Uuid], copy: bool, worn: &HashSet<Uuid>) -> Result<Mutation, String> {
    if root.is_nil() {
        return Err(aurora_i18n::tr!("marketplace-error-not-merchant").into());
    }
    let mut tree = Tree::new(inv, root, worn);
    let dest_depth = tree.depth(dest);
    if dest_depth < 0 || !tree.is_folder(dest) {
        return Err(aurora_i18n::tr!("marketplace-error-not-accepted").into());
    }
    if ids.is_empty() {
        return Err(aurora_i18n::tr!("world-inventory-actions-empty-selection").into());
    }
    let master = if dest == root {
        None
    } else {
        tree.first_descendant_of(root, dest).or(Some(dest))
    };
    let ids = crate::world::inventory::actions::roots(inv, ids);
    let bundle = ids.len();
    let mut audit = Vec::new();
    for id in &ids {
        if *id == root || tree.below(dest, *id) {
            return Err(aurora_i18n::tr!("marketplace-reason-cannot-move-root").into());
        }
        if tree.is_folder(*id) {
            let kind = tree.kind(*id);
            if !matches!(kind, -1 | STOCK) {
                return Err(aurora_i18n::tr!("marketplace-error-not-accepted").into());
            }
            tree.can_move_folder(master, dest, *id, bundle, true, copy)?;
            tree.permissions_for_sale(*id)?;
            if tree.kind(dest) == STOCK {
                return Err(aurora_i18n::tr!("marketplace-error-not-accepted").into());
            }
            if copy {
                let copied = tree.copy_folder(*id, dest, &mut HashSet::new())?;
                audit.push(copied);
            } else {
                tree.move_folder(*id, dest);
                audit.push(dest);
            }
        } else {
            tree.can_move_item(master, dest, *id, bundle, copy)?;
            let name = tree.name(*id);
            let mut to = dest;
            if dest_depth == 0 {
                let listing = tree.create(dest, -1, &name);
                to = tree.create(listing, -1, &name);
            } else if dest_depth == 1 {
                to = tree.create(dest, -1, &name);
            }
            if !copyable(inv, *id) && tree.kind(to) != STOCK {
                to = tree.create(to, STOCK, &name);
            }
            if copy {
                tree.copy_item(*id, to)?;
            } else {
                tree.move_item(*id, to);
            }
        }
    }
    let mut report = Report {
        ok: true,
        ..Default::default()
    };
    audit.dedup();
    for folder in audit {
        tree.validate(folder, true, -1, &mut report, 0);
    }
    Ok(tree.into_change())
}
