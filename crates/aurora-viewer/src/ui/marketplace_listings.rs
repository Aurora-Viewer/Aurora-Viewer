//! The "Marketplace listings" window: LLFloaterMarketplaceListings and
//! LLPanelMarketplaceListings (Firestorm indra/newview/
//! llfloatermarketplacelistings.cpp), the marketplace entries of the folder
//! menus (LLInvFVBridge::addMarketplaceContextMenuOptions and
//! LLFolderBridge::performAction in llinventorybridge.cpp), the labels of
//! LLMarketplaceFolderBridge, LLFloaterAssociateListing and
//! LLFloaterMarketplaceValidation, originally LGPL 2.1.

use super::icons::Icons;
use super::inventory::{FolderDrag, InvDrag, listings::Panel, tree};
use super::widgets::{self, ActionKind, Floater, Modal};
use crate::theme::Palette;
use crate::world::inventory::actions::{request_tree, under};
use crate::world::inventory::{FetchState, Inventory};
use crate::world::marketplace::validate::{self, Level, Message};
use crate::world::marketplace::{self as mp, Alert, Fetch, MarketplaceData, STOCK, STOCK_INFINITE, STOCK_NOT_EVALUATED, Status};
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::slm::{MerchantStatus, SlmError};
use egui::{Rect, RichText, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

pub const WINDOW: &str = "marketplace_listings";
mod commands;
pub use commands::{Command, OptionState};
const AUDIT_WINDOW: &str = "marketplace_validation";
/// MarketplaceURL_CreateStore (strings.xml): the knowledge base page on
/// opening a store, outside the Marketplace.
pub const CREATE_STORE_URL: &str =
    "https://community.secondlife.com/t5/English-Knowledge-Base/Selling-in-the-Marketplace/ta-p/700193#Section_.3";
/// floater_marketplace_listings.xml.
const DEFAULT_SIZE: Vec2 = Vec2::new(420.0, 560.0);
const MIN_SIZE: Vec2 = Vec2::new(340.0, 320.0);

/// The tabs: all listings, listed, unlisted, not associated with SLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    All,
    Listed,
    Unlisted,
    Unassociated,
}

const TABS: [Tab; 4] = [Tab::All, Tab::Listed, Tab::Unlisted, Tab::Unassociated];

/// What the window asks of the application.
#[derive(Debug)]
pub enum Action {
    /// A merchant without "Marketplace listings" folder: create it
    /// (LLFloaterMarketplaceListings::setRootFolder).
    CreateRoot,
    /// Inventory changes: a drop, the fixes of an audit, a new folder.
    Edit(Mutation),
    Paste {
        change: Mutation,
        cut: bool,
    },
    /// A Marketplace page, in the Marketplace window.
    OpenMarketplace(String),
    /// A page outside the Marketplace, in the system browser.
    OpenExternal(String),
    /// An alert for the notifications: title, text and a link.
    Notify {
        text: String,
        url: Option<String>,
    },
}

/// A choice the merchant confirms first.
#[derive(Debug, Clone)]
enum Confirm {
    /// ConfirmMerchantMoveInventory or ConfirmMerchantActiveChange.
    Drop {
        dest: Uuid,
        ids: Vec<Uuid>,
        active: bool,
    },
    /// ConfirmMerchantUnlist: associating a listed listing.
    Associate {
        folder: Uuid,
        id: i32,
    },
    Inventory {
        change: Mutation,
        cut: bool,
    },
    Paste {
        to: Uuid,
        ids: Vec<Uuid>,
        copy: bool,
        active: bool,
    },
}

#[derive(Default)]
pub struct ListingsUi {
    pub open: bool,
    pub data: MarketplaceData,
    tab: usize,
    filter: String,
    expanded: HashSet<Uuid>,
    /// LLFloaterMarketplaceValidation: the lines of the last audit.
    audit: Option<Vec<Message>>,
    audit_open: bool,
    /// LLFloaterAssociateListing: the folder and the id typed.
    associate: Option<(Uuid, String)>,
    confirm: Option<Confirm>,
    /// Why the last drop or action was refused.
    pub message: String,
    root_requested: bool,
    reconciled: u64,
    demo_audited: bool,
    demo_menu: Option<Uuid>,
}

/// isInActiveFolder: an object inside the version folder of a listed listing.
fn in_active_folder(data: &MarketplaceData, inv: &Inventory, root: Uuid, id: Uuid) -> bool {
    let depth = mp::depth(inv, root, id);
    let listing = mp::nested_parent(inv, id, depth);
    let version = data.version_folder(listing);
    data.activation_state(listing) && !version.is_nil() && under(inv, id, version)
}

/// How a badge reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Muted,
    Good,
    Bad,
    Busy,
}

/// The listing badges of a folder (LLMarketplaceFolderBridge::getLabelSuffix):
/// its listing ID, listed, the active version, the stock, an update running.
fn badges(data: &MarketplaceData, inv: &Inventory, root: Uuid, id: Uuid) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    if let Some(r) = data.record(id) {
        let id_text = if r.id == 0 {
            aurora_i18n::tr!("marketplace-suffix-no-id").to_owned()
        } else {
            format!("#{}", r.id)
        };
        out.push((id_text, Tone::Muted));
        if r.listed {
            out.push((aurora_i18n::tr!("marketplace-suffix-listed").to_owned(), Tone::Good));
        }
    } else if data.is_version_folder(id) {
        out.push((aurora_i18n::tr!("marketplace-suffix-active").to_owned(), Tone::Good));
    }
    let count = data.stock_count(inv, root, id, false);
    if count == 0 {
        out.push((aurora_i18n::tr!("marketplace-suffix-out-of-stock").to_owned(), Tone::Bad));
    } else if count != STOCK_INFINITE {
        let label = if inv.folders.get(&id).is_some_and(|f| f.info.type_default == STOCK) {
            aurora_i18n::tr!("marketplace-suffix-stock")
        } else {
            aurora_i18n::tr!("marketplace-suffix-max")
        };
        if count == STOCK_NOT_EVALUATED {
            out.push((format!("{label}={}", aurora_i18n::tr!("marketplace-suffix-updating")), Tone::Busy));
        } else {
            out.push((format!("{label}={count}"), Tone::Muted));
        }
    }
    if data.is_updating(inv, root, id) {
        out.push((aurora_i18n::tr!("marketplace-suffix-updating").to_owned(), Tone::Busy));
    }
    out
}

/// A row label: the name in the inventory's row font, then the badges in the
/// font and colours of the inventory's permission badges.
fn row_label(name: &str, badges: &[(String, Tone)], p: &Palette) -> egui::text::LayoutJob {
    let mut job = tree::name(name, p);
    let font = egui::FontId::proportional(12.0);
    for (i, (text, tone)) in badges.iter().enumerate() {
        let gap = if i == 0 { "  " } else { " · " };
        job.append(gap, 0.0, egui::TextFormat::simple(font.clone(), p.muted_dim));
        let color = match tone {
            Tone::Muted => p.muted,
            Tone::Good => p.success,
            Tone::Bad => p.danger,
            Tone::Busy => p.amber,
        };
        job.append(text, 0.0, egui::TextFormat::simple(font.clone(), color));
    }
    job
}

/// LLMarketplaceData::getMarketplaceStringSubstitutions' dashboard.
fn dashboard(home: &str) -> String {
    format!("{}/merchants/store/dashboard", home.trim_end_matches('/'))
}

/// The text of an alert of LLMarketplaceData (notifications.xml).
pub fn alert_text(alert: &Alert) -> (String, Option<String>) {
    match alert {
        Alert::Failed(SlmError::Unprocessable) => (aurora_i18n::tr!("marketplace-alert-unprocessable").into(), None),
        Alert::Failed(SlmError::Transaction { reason, description }) => (
            aurora_i18n::tr!(
                "marketplace-alert-transaction",
                reason = reason.trim().to_owned(),
                description = description.clone()
            ),
            None,
        ),
        Alert::NotUpdated { url } => (
            aurora_i18n::tr!("marketplace-alert-not-updated").into(),
            Some(url.clone()).filter(|u| !u.is_empty()),
        ),
        Alert::ActivateRequired => (aurora_i18n::tr!("marketplace-alert-activate-required").into(), None),
        Alert::VersionEmpty => (aurora_i18n::tr!("marketplace-alert-version-empty").into(), None),
    }
}

impl ListingsUi {
    /// LLFloaterMarketplaceListings::onOpen; true when GET /merchant must
    /// be sent (initializeSLM).
    pub fn open(&mut self) -> bool {
        log::info!("marketplace listings window opened");
        self.open = true;
        self.data.initialize()
    }

    /// Every frame, open or not: the listings follow the inventory
    /// (update_marketplace_category), once the listings are known.
    pub fn reconcile(&mut self, inv: &Inventory) {
        if inv.generation == self.reconciled || self.data.fetch() != Fetch::Done {
            return;
        }
        self.reconciled = inv.generation;
        if let Some(root) = mp::root(inv) {
            self.data.reconcile(inv, root);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        icons: &Icons,
        inv: &mut Inventory,
        worn: &HashSet<Uuid>,
        home: Option<&str>,
        panel: &mut Panel<'_>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        panel.begin();
        self.expanded.extend(panel.reveal());
        let work = super::layout::work_area(ctx);
        let size = DEFAULT_SIZE.min(work.size() - Vec2::splat(2.0 * super::layout::GAP)).max(MIN_SIZE);
        let default = Rect::from_min_size((work.center() - size * 0.5).round(), size);
        let mut floater = Floater::new(WINDOW, aurora_i18n::tr!("marketplace-listings-title"), default.min, default.size());
        floater.min_size = MIN_SIZE;
        let mut open = self.open;
        floater.show(ctx, p, &mut open, |ui| {
            self.contents(ui, p, icons, inv, worn, home, panel, &mut actions)
        });
        self.open = open;
        if !open {
            panel.begin();
        }
        self.dialogs(ctx, p, inv, worn, &panel.prefs.protected, &mut actions);
        self.audit_window(ctx, p);
        if let Some(id) = self.demo_menu {
            let mut open = true;
            super::menu::popup_at(
                ctx,
                egui::Id::unique("listings_demo_menu"),
                work.center() - Vec2::new(20.0, 180.0),
                &mut open,
                p,
                |ui| {
                    panel
                        .state
                        .marketplace_commands
                        .insert(id, self.inventory_commands(inv, mp::root(inv).unwrap_or_default(), id));
                    panel.menu(ui, p, inv, id);
                },
            );
            if !open {
                self.demo_menu = None;
            }
        }
        panel.finish(ctx, inv);
        actions
    }

    /// LLFloaterMarketplaceListings::updateView.
    #[allow(clippy::too_many_arguments)]
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        icons: &Icons,
        inv: &mut Inventory,
        worn: &HashSet<Uuid>,
        home: Option<&str>,
        panel: &mut Panel<'_>,
        actions: &mut Vec<Action>,
    ) {
        let merchant = self.data.is_merchant();
        let root = mp::root(inv);
        if merchant && root.is_none() && !inv.root.is_nil() && !self.root_requested {
            self.root_requested = true;
            actions.push(Action::CreateRoot);
        }
        if let (true, Some(root)) = (merchant, root) {
            self.data.fetch_contents(root);
            if inv.folders.get(&root).is_some_and(|f| f.state != FetchState::Fetched) || ui.ctx().cumulative_pass_nr().is_multiple_of(30) {
                // LLInventoryModelBackgroundFetch::start(mRootFolderId)
                request_tree(inv, root);
            }
        }
        let status = self.data.status().clone();
        let loading = matches!(status, Status::NotInitialized | Status::Initializing)
            || (merchant && (root.is_none() || matches!(self.data.fetch(), Fetch::NotDone | Fetch::Loading)));
        if loading {
            placeholder(
                ui,
                p,
                aurora_i18n::tr!("marketplace-listings-initializing-title"),
                aurora_i18n::tr!("marketplace-listings-initializing"),
            );
            ui.vertical_centered(|ui| ui.spinner());
            return;
        }
        if let Some(root) = root.filter(|_| merchant) {
            // the inventory window's order: filter, tabs, tree, footer
            let clear = !self.filter.is_empty();
            let hint = aurora_i18n::tr!("marketplace-listings-filter");
            let width = ui.available_width();
            let (_, cleared) = tree::search_field(ui, p, &mut self.filter, hint, width, clear, aurora_i18n::tr!("common-clear"));
            if cleared {
                self.filter.clear();
            }
            ui.add_space(3.0);
            let labels = [
                (aurora_i18n::tr!("marketplace-listings-tab-all"), true),
                (aurora_i18n::tr!("marketplace-listings-tab-listed"), true),
                (aurora_i18n::tr!("marketplace-listings-tab-unlisted"), true),
                (aurora_i18n::tr!("marketplace-listings-tab-unassociated"), true),
            ];
            widgets::tabs(ui, p, &mut self.tab, &labels);
            ui.add_space(6.0);
            let count = self.listings_tree(ui, p, icons, inv, worn, root, panel, actions);
            self.footer(ui, p, inv, worn, root, home, panel, count, actions);
            return;
        }
        match status {
            Status::Known(MerchantStatus::ConnectionFailure(reason)) => {
                let text = if reason.is_empty() {
                    aurora_i18n::tr!("marketplace-listings-connection-error").to_owned()
                } else {
                    aurora_i18n::tr!("marketplace-listings-connection-error-reason", reason = reason)
                };
                placeholder(ui, p, aurora_i18n::tr!("marketplace-listings-error-title"), &text);
            }
            Status::Known(MerchantStatus::NotMerchant) => {
                placeholder(
                    ui,
                    p,
                    aurora_i18n::tr!("marketplace-listings-not-merchant-title"),
                    aurora_i18n::tr!("marketplace-listings-not-merchant"),
                );
                ui.vertical_centered(|ui| {
                    let label = aurora_i18n::tr!("marketplace-listings-create-store");
                    if widgets::action_button(ui, p, "building-store", label, ActionKind::Primary, Vec2::new(0.0, 28.0)).clicked() {
                        actions.push(Action::OpenExternal(CREATE_STORE_URL.into()));
                    }
                });
            }
            _ => placeholder(
                ui,
                p,
                aurora_i18n::tr!("marketplace-listings-error-title"),
                aurora_i18n::tr!("marketplace-listings-error"),
            ),
        }
    }

    /// The listings of the tab, in the inventory's list frame; the number
    /// shown.
    #[allow(clippy::too_many_arguments)]
    fn listings_tree(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        icons: &Icons,
        inv: &mut Inventory,
        worn: &HashSet<Uuid>,
        root: Uuid,
        panel: &mut Panel<'_>,
        actions: &mut Vec<Action>,
    ) -> usize {
        let tab = TABS[self.tab.min(3)];
        // allowDropOnRoot: only the All and Unassociated tabs take a drop
        let root_drop = matches!(tab, Tab::All | Tab::Unassociated);
        let filter = self.filter.trim().to_lowercase();
        let any = !mp::subfolders(inv, root).is_empty();
        let listings: Vec<Uuid> = {
            let mut l: Vec<(String, Uuid)> = mp::subfolders(inv, root)
                .iter()
                .filter(|id| match tab {
                    Tab::All => true,
                    Tab::Listed => self.data.activation_state(**id),
                    Tab::Unlisted => self.data.is_listed(**id) && !self.data.activation_state(**id),
                    Tab::Unassociated => !self.data.is_listed(**id),
                })
                .filter(|id| filter.is_empty() || matches(inv, **id, &filter, 0))
                .map(|id| (inv.folders[id].info.name.to_lowercase(), *id))
                .collect();
            l.sort();
            l.into_iter().map(|(_, id)| id).collect()
        };
        let count = listings.len();
        let list_h = tree::height(ui);
        let zone = tree::frame(ui, p, |ui| {
            panel.focus(ui, list_h);
            tree::scroll(ui, list_h).show(ui, |ui| {
                tree::spacing(ui);
                if !any {
                    // InventoryMarketplaceListingsNoItems
                    ui.label(
                        RichText::new(aurora_i18n::tr!("marketplace-listings-empty-title"))
                            .size(13.0)
                            .color(p.ink),
                    );
                    ui.label(RichText::new(aurora_i18n::tr!("marketplace-listings-empty")).color(p.muted));
                } else if listings.is_empty() {
                    let text = if filter.is_empty() {
                        aurora_i18n::tr!("marketplace-listings-no-listing")
                    } else {
                        aurora_i18n::tr!("marketplace-listings-no-match")
                    };
                    ui.label(RichText::new(text).color(p.muted));
                }
                for id in listings {
                    self.folder_tree(ui, p, icons, inv, worn, root, id, 1, &filter, panel, actions);
                }
                if root_drop && any {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(aurora_i18n::tr!("marketplace-listings-drop"))
                            .size(11.0)
                            .color(p.muted_dim),
                    );
                }
            });
        });
        panel.shortcuts(ui, inv, root);
        if root_drop {
            self.drop_target(ui, p, &zone.response, inv, worn, root, root, actions);
        } else if zone.response.contains_pointer()
            && (zone.response.dnd_release_payload::<InvDrag>().is_some() || zone.response.dnd_release_payload::<FolderDrag>().is_some())
        {
            // TooltipOutboxCannotDropOnRoot
            self.message = aurora_i18n::tr!("marketplace-reason-cannot-drop-on-root").into();
        }
        count
    }

    /// The footer of the inventory window: the actions on the left, the
    /// count or the last message on the right.
    #[allow(clippy::too_many_arguments)]
    fn footer(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        inv: &Inventory,
        worn: &HashSet<Uuid>,
        root: Uuid,
        home: Option<&str>,
        panel: &mut Panel<'_>,
        count: usize,
        actions: &mut Vec<Action>,
    ) {
        ui.horizontal(|ui| {
            // LLPanelMarketplaceListings::onAddButtonClicked
            if widgets::icon_button(ui, p, "plus", aurora_i18n::tr!("marketplace-listings-new-folder"), true) {
                let id = Uuid::new_v4();
                actions.push(Action::Edit(Mutation {
                    operations: vec![Operation::Folder {
                        id,
                        parent: root,
                        name: aurora_i18n::tr!("marketplace-listings-new-folder-name").into(),
                        type_default: -1,
                    }],
                    refresh: vec![root, id],
                    ..Default::default()
                }));
                panel.state.selected = Some(id);
                panel.state.selection.clear();
                panel.state.selection.insert(id);
            }
            // onAuditButtonClicked: the whole listings, nothing changed
            if widgets::icon_button(ui, p, "clipboard-text-filled", aurora_i18n::tr!("marketplace-listings-audit"), true) {
                self.run_audit(inv, worn, root, root);
            }
            if let Some(home) = home
                && widgets::icon_button(ui, p, "dashboard-filled", aurora_i18n::tr!("marketplace-listings-dashboard"), true)
            {
                actions.push(Action::OpenMarketplace(dashboard(home)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.message.is_empty() {
                    tree::status_text(ui, &aurora_i18n::tr!("marketplace-listings-count", count = count), p.muted);
                } else {
                    tree::status_text(ui, &self.message, p.warn);
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn folder_tree(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        icons: &Icons,
        inv: &mut Inventory,
        worn: &HashSet<Uuid>,
        root: Uuid,
        id: Uuid,
        depth: i32,
        filter: &str,
        panel: &mut Panel<'_>,
        actions: &mut Vec<Action>,
    ) {
        tree::spacing(ui);
        if depth > 8 {
            return;
        }
        let Some(folder) = inv.folders.get(&id) else { return };
        let name = folder.info.name.clone();
        let kind = folder.info.type_default;
        let state = folder.state;
        let open = self.expanded.contains(&id) || !filter.is_empty();
        let active = self.data.activation_state(id) && depth == 1;
        let label = row_label(&name, &badges(&self.data, inv, root, id), p);
        let selected = panel.state.selection.contains(&id);
        let mut renaming = false;
        let response = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if tree::caret(ui, p, open).clicked() {
                    self.toggle(id);
                }
                // FT_MARKETPLACE_STOCK, and FT_MARKETPLACE_VERSION for the
                // active version folder (LLMarketplaceFolderBridge::getIcon)
                let (icon, tint) = if kind == STOCK {
                    ("package", Some(p.amber))
                } else if depth == 2 && self.data.is_version_folder(id) {
                    (tree::folder_icon(-1, open), Some(p.success))
                } else {
                    (tree::folder_icon(-1, open), None)
                };
                tree::icon(ui, icons, icon, p, tint);
                // getLabelStyle: bold while listed
                let bold = active.then(|| ui.painter().layout_no_wrap(name.clone(), tree::font(), p.ink));
                renaming = panel.rename(ui, p, inv, id);
                if renaming {
                    ui.response()
                } else {
                    tree::label(ui, label, selected, egui::Sense::click_and_drag(), bold)
                }
            })
            .inner;
        if !renaming {
            response.dnd_set_drag_payload(FolderDrag(id));
            panel.interact(ui, inv, id, &response);
            if response.double_clicked() {
                self.toggle(id);
            }
            self.drop_target(ui, p, &response, inv, worn, root, id, actions);
            super::menu::context_menu(&response, p, |ui| {
                panel.state.marketplace_commands.insert(id, self.inventory_commands(inv, root, id));
                let pending = panel.state.pending;
                if self.data.is_updating(inv, root, id) {
                    panel.state.pending = Some(Uuid::nil());
                }
                panel.menu(ui, p, inv, id);
                panel.state.pending = pending;
            });
        }
        if !open {
            return;
        }
        if matches!(state, FetchState::Unknown | FetchState::Failed) {
            inv.request(id);
        }
        ui.indent(("marketplace_listing", id), |ui| {
            let mut children: Vec<(String, Uuid)> = mp::subfolders(inv, id)
                .iter()
                .filter(|c| filter.is_empty() || matches(inv, **c, filter, 0))
                .map(|c| (inv.folders[c].info.name.to_lowercase(), *c))
                .collect();
            children.sort();
            let empty = children.is_empty() && inv.folders.get(&id).is_none_or(|f| f.items.is_empty());
            for (_, child) in children {
                self.folder_tree(ui, p, icons, inv, worn, root, child, depth + 1, filter, panel, actions);
            }
            let items: Vec<Uuid> = inv.folders.get(&id).map(|f| f.items.clone()).unwrap_or_default();
            for item in items {
                let Some(it) = inv.items.get(&item) else { continue };
                if !filter.is_empty() && !it.name.to_lowercase().contains(filter) {
                    continue;
                }
                let (icon, tint) = super::inventory::inventory_item_icon(inv, it, p);
                let mut label = tree::name(&it.name, p);
                if let Some(permissions) = super::inventory::permissions::Permissions::inventory(inv, item) {
                    permissions.append_suffix(ui, &mut label, p);
                }
                let selected = panel.state.selection.contains(&item);
                let mut renaming = false;
                let response = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        tree::icon(ui, icons, icon, p, Some(tint));
                        renaming = panel.rename(ui, p, inv, item);
                        if renaming {
                            ui.response()
                        } else {
                            tree::label(ui, label, selected, egui::Sense::click_and_drag(), None)
                        }
                    })
                    .inner;
                if renaming {
                    continue;
                }
                response.dnd_set_drag_payload(InvDrag(item));
                panel.interact(ui, inv, item, &response);
                super::menu::context_menu(&response, p, |ui| {
                    panel
                        .state
                        .marketplace_commands
                        .insert(item, self.inventory_commands(inv, root, item));
                    let pending = panel.state.pending;
                    if self.data.is_updating(inv, root, item) {
                        panel.state.pending = Some(Uuid::nil());
                    }
                    panel.menu(ui, p, inv, item);
                    panel.state.pending = pending;
                });
                // a drop on an item goes to its folder
                self.drop_target(ui, p, &response, inv, worn, root, id, actions);
            }
            if empty {
                let text = if state == FetchState::Fetched {
                    aurora_i18n::tr!("status-empty")
                } else {
                    aurora_i18n::tr!("ui-inventory-loading-folder")
                };
                ui.label(RichText::new(text).size(12.0).color(p.muted_dim));
            }
        });
    }

    fn toggle(&mut self, id: Uuid) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
    }

    /// marketplace_create_listing: an audit, then one that fixes, then the
    /// listing.
    fn create_listing(&mut self, inv: &Inventory, worn: &HashSet<Uuid>, root: Uuid, id: Uuid, actions: &mut Vec<Action>) {
        let first = validate::validate(inv, root, id, false, -1, worn);
        if first.ok {
            self.data.create_listing(inv, root, id);
            return;
        }
        let fixed = validate::validate(inv, root, id, true, -1, worn);
        if fixed.split {
            actions.push(notify(aurora_i18n::tr!("marketplace-alert-stock-split")));
        }
        let ok = fixed.ok;
        let errors = fixed.errors();
        if !fixed.change.operations.is_empty() {
            actions.push(Action::Edit(fixed.change));
        }
        if ok {
            actions.push(notify(aurora_i18n::tr!("marketplace-alert-force-validate")));
            // the listing is created once the fixed folders are known
            // (Firestorm creates it at once, before AIS answers)
            self.data.create_listing(inv, root, id);
        } else {
            let error = if errors.is_empty() { first.errors() } else { errors };
            actions.push(notify(&aurora_i18n::tr!("marketplace-alert-listing-failed", error = error)));
        }
    }

    /// marketplace_list: the version folder must pass the audit.
    fn list(&mut self, inv: &Inventory, worn: &HashSet<Uuid>, root: Uuid, id: Uuid, actions: &mut Vec<Action>) {
        let version = self.data.version_folder(id);
        let report = validate::validate(inv, root, version, false, -1, worn);
        if report.ok {
            self.data.activate_listing(inv, root, id, true);
        } else {
            actions.push(notify(&aurora_i18n::tr!(
                "marketplace-alert-listing-failed",
                error = report.errors()
            )));
        }
    }

    /// marketplace_activate: a version folder that passes the audit.
    fn activate(&mut self, inv: &Inventory, worn: &HashSet<Uuid>, root: Uuid, id: Uuid, actions: &mut Vec<Action>) {
        let report = validate::validate(inv, root, id, false, 2, worn);
        if report.ok {
            let parent = inv.folders.get(&id).map_or(Uuid::nil(), |f| f.info.parent);
            self.data.set_version_folder(inv, root, parent, id);
        } else {
            actions.push(notify(&aurora_i18n::tr!(
                "marketplace-alert-activation-failed",
                error = report.errors()
            )));
        }
    }

    fn run_audit(&mut self, inv: &Inventory, worn: &HashSet<Uuid>, root: Uuid, folder: Uuid) {
        let report = validate::validate(inv, root, folder, false, -1, worn);
        self.audit = Some(report.shown());
        self.audit_open = true;
    }

    /// A row (or the root area) that takes inventory dropped on it.
    #[allow(clippy::too_many_arguments)]
    fn drop_target(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        r: &egui::Response,
        inv: &Inventory,
        worn: &HashSet<Uuid>,
        root: Uuid,
        dest: Uuid,
        actions: &mut Vec<Action>,
    ) {
        let cargo = r
            .dnd_hover_payload::<InvDrag>()
            .map(|d| d.0)
            .or_else(|| r.dnd_hover_payload::<FolderDrag>().map(|d| d.0));
        let Some(id) = cargo else { return };
        let plan = validate::drop_plan(inv, root, dest, &[id], worn);
        // the inventory's drop feedback: outline and cursor
        tree::drop_outline(ui, p, r.rect, plan.is_ok());
        ui.ctx().set_cursor_icon(if plan.is_ok() {
            egui::CursorIcon::Move
        } else {
            egui::CursorIcon::NotAllowed
        });
        let released = r.dnd_release_payload::<InvDrag>().is_some() || r.dnd_release_payload::<FolderDrag>().is_some();
        if !released {
            return;
        }
        match plan {
            Ok(_) => {
                let from_listings = mp::depth(inv, root, id) >= 0;
                let active =
                    in_active_folder(&self.data, inv, root, dest) || (from_listings && in_active_folder(&self.data, inv, root, id));
                if active || !from_listings {
                    self.confirm = Some(Confirm::Drop {
                        dest,
                        ids: vec![id],
                        active,
                    });
                } else {
                    self.apply_drop(inv, worn, root, dest, &[id], actions);
                }
            }
            Err(reason) => {
                self.message.clone_from(&reason);
                actions.push(notify(&aurora_i18n::tr!("marketplace-alert-paste-failed", error = reason)));
            }
        }
    }

    fn apply_drop(&mut self, inv: &Inventory, worn: &HashSet<Uuid>, root: Uuid, dest: Uuid, ids: &[Uuid], actions: &mut Vec<Action>) {
        match validate::drop_plan(inv, root, dest, ids, worn) {
            Ok(change) => {
                self.message.clear();
                self.expanded.insert(dest);
                actions.push(Action::Edit(change));
            }
            Err(reason) => self.message = reason,
        }
    }

    /// The confirmations and LLFloaterAssociateListing.
    #[allow(clippy::too_many_arguments)]
    fn dialogs(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        inv: &Inventory,
        worn: &HashSet<Uuid>,
        protected: &HashSet<Uuid>,
        actions: &mut Vec<Action>,
    ) {
        let Some(root) = mp::root(inv) else {
            self.confirm = None;
            self.associate = None;
            return;
        };
        if let Some(confirm) = self.confirm.clone() {
            let text = match &confirm {
                Confirm::Drop { active: true, .. } | Confirm::Paste { active: true, .. } | Confirm::Inventory { .. } => {
                    aurora_i18n::tr!("marketplace-confirm-active-change")
                }
                Confirm::Drop { .. } | Confirm::Paste { .. } => aurora_i18n::tr!("marketplace-confirm-move"),
                Confirm::Associate { .. } => aurora_i18n::tr!("marketplace-confirm-unlist"),
            };
            let mut answer = None;
            Modal::new(egui::Id::unique("marketplace_listings_confirm")).show(ctx, |ui| {
                ui.set_max_width(380.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("marketplace-listings-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(RichText::new(text).color(p.ink));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "check",
                        aurora_i18n::tr!("common-ok"),
                        ActionKind::Primary,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "x",
                        aurora_i18n::tr!("common-cancel"),
                        ActionKind::Flat,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                    {
                        answer = Some(false);
                    }
                });
            });
            if let Some(yes) = answer {
                self.confirm = None;
                if yes {
                    match confirm {
                        Confirm::Drop { dest, ids, .. } => self.apply_drop(inv, worn, root, dest, &ids, actions),
                        Confirm::Associate { folder, id } => {
                            let source = self.data.listing_folder(id);
                            self.data.associate_listing(inv, folder, source, id);
                        }
                        Confirm::Inventory { change, cut } => actions.push(Action::Paste { change, cut }),
                        Confirm::Paste { to, ids, copy, .. } => self.paste_inventory(inv, worn, protected, to, &ids, copy, true, actions),
                    }
                }
            }
            return;
        }
        let Some((folder, mut text)) = self.associate.take() else { return };
        let mut answer = None;
        Modal::new(egui::Id::unique("marketplace_associate_listing")).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("marketplace-associate-title"))
                    .size(15.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(aurora_i18n::tr!("marketplace-associate-prompt")).color(p.ink));
            let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.0));
            // validateNonNegativeS32
            text.retain(|c| c.is_ascii_digit());
            text.truncate(10);
            if edit.lost_focus() && widgets::enter_pressed(ui) {
                answer = Some(true);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "link",
                    aurora_i18n::tr!("common-ok"),
                    ActionKind::Primary,
                    Vec2::new(0.0, 28.0),
                )
                .clicked()
                {
                    answer = Some(true);
                }
                if widgets::action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("common-cancel"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 28.0),
                )
                .clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    answer = Some(false);
                }
            });
        });
        match answer {
            None => self.associate = Some((folder, text)),
            Some(false) => {}
            Some(true) => match text.parse::<i32>().ok().filter(|id| *id > 0) {
                // LLFloaterAssociateListing::apply
                Some(id) => {
                    let source = self.data.listing_folder(id);
                    if !source.is_nil() && self.data.activation_state(source) && mp::subfolders(inv, folder).len() != 1 {
                        self.confirm = Some(Confirm::Associate { folder, id });
                    } else {
                        self.data.associate_listing(inv, folder, source, id);
                    }
                }
                None => actions.push(notify(aurora_i18n::tr!("marketplace-associate-invalid"))),
            },
        }
    }

    /// LLFloaterMarketplaceValidation.
    fn audit_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.audit_open {
            return;
        }
        let Some(lines) = &self.audit else { return };
        let work = super::layout::work_area(ctx);
        let size = Vec2::new(460.0, 360.0).min(work.size());
        let pos = (work.center() - size * 0.5).round();
        let mut open = true;
        Floater::new(AUDIT_WINDOW, aurora_i18n::tr!("marketplace-audit-title"), pos, size).show(ctx, p, &mut open, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if lines.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("marketplace-check-ok")).color(p.ink));
                }
                for line in lines {
                    // errors in bold, like the validation floater
                    let mut text = RichText::new(format!("{}{}", "    ".repeat(line.indent), line.text)).size(12.5);
                    text = match line.level {
                        Level::Error => text.strong().color(p.danger),
                        Level::Warn => text.color(p.warn),
                        Level::Info => text.color(p.ink),
                    };
                    ui.label(text);
                }
            });
        });
        self.audit_open = open;
    }

    /// The window in the state the demo asks for (docs/TESTING.md).
    pub fn demo_audit(&mut self, inv: &Inventory, worn: &HashSet<Uuid>) {
        if self.demo_audited || self.data.fetch() != Fetch::Done {
            return;
        }
        if let Some(root) = mp::root(inv) {
            self.demo_audited = true;
            self.run_audit(inv, worn, root, root);
        }
    }

    pub fn demo_expand(&mut self, ids: &[Uuid]) {
        self.expanded.extend(ids.iter().copied());
    }

    pub fn demo_menu(&mut self, id: Uuid) {
        self.demo_menu = Some(id);
    }
}

fn notify(text: &str) -> Action {
    Action::Notify {
        text: text.to_owned(),
        url: None,
    }
}

/// A name, or one below it, contains the filter.
fn matches(inv: &Inventory, id: Uuid, filter: &str, guard: usize) -> bool {
    let Some(f) = inv.folders.get(&id) else { return false };
    if f.info.name.to_lowercase().contains(filter) {
        return true;
    }
    guard < 8
        && (f
            .items
            .iter()
            .any(|i| inv.items.get(i).is_some_and(|it| it.name.to_lowercase().contains(filter)))
            || f.children.iter().any(|c| matches(inv, *c, filter, guard + 1)))
}

fn placeholder(ui: &mut egui::Ui, p: &Palette, title: &str, text: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(14.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(12.5).color(p.muted));
    });
    ui.add_space(12.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::marketplace::demo;
    use std::time::Instant;

    fn demo_inventory() -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: inv.root,
                    name: "My Inventory".into(),
                    type_default: 8,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        demo::seed(&mut inv, Uuid::from_u128(2));
        inv
    }

    fn merchant(inv: &Inventory) -> ListingsUi {
        let mut ui = ListingsUi::default();
        assert!(ui.open());
        let mut slm = demo::DemoSlm::new(demo::Mode::Merchant);
        ui.data.set_status(slm.merchant());
        let root = mp::root(inv).unwrap();
        ui.data.fetch_contents(root);
        for request in ui.data.take_requests(std::time::Instant::now()) {
            let reply = slm.answer(&request);
            ui.data.apply(inv, root, &request, reply, std::time::Instant::now());
        }
        ui
    }

    #[test]
    fn labels_follow_the_listing_states() {
        let inv = demo_inventory();
        let ui = merchant(&inv);
        let root = mp::root(&inv).unwrap();
        let text = |id| badges(&ui.data, &inv, root, id).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        let hoodie = badges(&ui.data, &inv, root, demo::HOODIE);
        assert_eq!(hoodie[0], ("#4101".to_owned(), Tone::Muted));
        assert_eq!(hoodie[1], (aurora_i18n::tr!("marketplace-suffix-listed").to_owned(), Tone::Good));
        assert!(text(demo::HOODIE_V2).contains(&aurora_i18n::tr!("marketplace-suffix-active").to_owned()));
        assert!(text(demo::POSTER).iter().any(|t| t.ends_with("=3")), "{:?}", text(demo::POSTER));
        assert!(text(demo::LANTERN).is_empty());
        assert!(in_active_folder(&ui.data, &inv, root, Uuid::from_u128(8_400_102)));
        assert!(!in_active_folder(&ui.data, &inv, root, Uuid::from_u128(8_400_101)));
    }

    #[test]
    fn inventory_commands_recheck_merchant_loading_and_pending_listing_updates() {
        let inv = demo_inventory();
        let root = mp::root(&inv).expect("listings root");
        let mut ui = ListingsUi::default();
        assert!(ui.inventory_commands(&inv, root, demo::HOODIE).iter().all(|option| !option.enabled));
        let mut actions = Vec::new();
        ui.inventory_command(&inv, &HashSet::new(), root, demo::HOODIE, Command::Unlist, &mut actions);
        assert!(actions.is_empty());
        assert!(ui.data.take_requests(Instant::now()).is_empty());
        ui = merchant(&inv);
        assert!(
            ui.inventory_commands(&inv, root, demo::HOODIE)
                .iter()
                .any(|option| option.command == Command::Unlist && option.enabled)
        );
        ui.inventory_command(&inv, &HashSet::new(), root, demo::HOODIE, Command::Unlist, &mut actions);
        assert!(ui.inventory_commands(&inv, root, demo::HOODIE).iter().all(|option| !option.enabled));
        assert!(
            ui.data
                .take_requests(Instant::now())
                .iter()
                .any(|request| matches!(request, aurora_net::slm::SlmRequest::Update { .. }))
        );
    }

    #[test]
    fn item_rows_only_edit_the_listing_and_active_structural_changes_ask_first() {
        let inv = demo_inventory();
        let mut ui = merchant(&inv);
        let root = mp::root(&inv).expect("listings root");
        let item = Uuid::from_u128(8_400_102);
        let commands = ui.inventory_commands(&inv, root, item);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command, Command::Edit);
        let change =
            crate::world::inventory::actions::patch(&inv, item, aurora_llsd::llsd_map! { "name" => "Renamed" }).expect("name edit");
        assert!(ui.guard_inventory_change(&inv, change, false).is_some());
        let change = crate::world::inventory::actions::patch(&inv, item, aurora_llsd::llsd_map! { "parent_id" => inv.root }).expect("move");
        assert!(ui.guard_inventory_change(&inv, change, false).is_none());
        assert!(matches!(ui.confirm, Some(Confirm::Inventory { .. })));
        assert!(ui.open);
    }

    #[test]
    fn clipboard_confirmation_rechecks_protection_and_does_not_consume_a_refused_cut() {
        let inv = demo_inventory();
        let mut ui = merchant(&inv);
        let item = Uuid::from_u128(8_400_102);
        let worn = HashSet::new();
        let mut actions = Vec::new();
        ui.paste_inventory(&inv, &worn, &HashSet::new(), demo::HOODIE_V2, &[item], true, false, &mut actions);
        assert!(actions.is_empty());
        assert!(matches!(
            ui.confirm,
            Some(Confirm::Paste {
                active: true,
                copy: true,
                ..
            })
        ));
        ui.confirm = None;
        // a lock no longer closes a folder to moves (#1728, rule B): what the
        // confirmation still re-checks is a locked entry, which cannot move
        ui.paste_inventory(
            &inv,
            &worn,
            &HashSet::from([item]),
            demo::HOODIE_V1,
            &[item],
            false,
            true,
            &mut actions,
        );
        assert!(actions.iter().all(|action| !matches!(action, Action::Paste { .. })));
        actions.clear();
        ui.paste_inventory(&inv, &worn, &HashSet::new(), demo::HOODIE_V1, &[item], false, true, &mut actions);
        assert!(
            matches!(actions.as_slice(), [Action::Paste { cut: true, .. }]),
            "the same cut without the lock"
        );
        actions.clear();
        ui.paste_inventory(&inv, &worn, &HashSet::new(), demo::HOODIE_V1, &[item], true, false, &mut actions);
        assert!(matches!(actions.as_slice(), [Action::Paste { cut: false, .. }]));
    }

    #[test]
    fn the_lantern_audit_reports_its_loose_items() {
        let inv = demo_inventory();
        let mut ui = merchant(&inv);
        let root = mp::root(&inv).unwrap();
        ui.run_audit(&inv, &HashSet::new(), root, root);
        let lines = ui.audit.clone().unwrap();
        let lantern = aurora_i18n::tr!("marketplace-demo-lantern");
        assert!(
            lines.iter().any(|l| l.level == Level::Error && l.text.contains(lantern)),
            "{lines:?}"
        );
        // the clean listings are left out
        assert!(
            !lines.iter().any(|l| l.text.contains(aurora_i18n::tr!("marketplace-demo-hoodie"))),
            "{lines:?}"
        );
    }

    #[test]
    fn creating_a_listing_fixes_then_asks_slm() {
        let inv = demo_inventory();
        let mut ui = merchant(&inv);
        let root = mp::root(&inv).unwrap();
        let mut actions = Vec::new();
        ui.create_listing(&inv, &HashSet::new(), root, demo::LANTERN, &mut actions);
        assert!(actions.iter().any(|a| matches!(a, Action::Edit(_))), "{actions:?}");
        let requests = ui.data.take_requests(std::time::Instant::now());
        assert!(matches!(requests[..], [aurora_net::slm::SlmRequest::Create { folder, .. }] if folder == demo::LANTERN));
    }

    #[test]
    fn alerts_read_like_firestorm() {
        let (text, url) = alert_text(&Alert::NotUpdated { url: "https://m/1".into() });
        assert!(!text.is_empty());
        assert_eq!(url.as_deref(), Some("https://m/1"));
        let (text, _) = alert_text(&Alert::Failed(SlmError::Transaction {
            reason: "'Bad Request'\n".into(),
            description: "no price".into(),
        }));
        assert!(text.contains("'Bad Request'") && text.contains("no price"), "{text}");
        assert_eq!(
            dashboard("https://marketplace.secondlife.com/"),
            "https://marketplace.secondlife.com/merchants/store/dashboard"
        );
    }
}
