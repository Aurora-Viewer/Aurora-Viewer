//! Shared listing commands for both inventory menus and the listings window.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Create,
    Associate,
    Check,
    List,
    Unlist,
    Disassociate,
    Activate,
    Deactivate,
    Edit,
    Refresh,
}

#[derive(Debug, Clone)]
pub struct OptionState {
    pub command: Command,
    pub enabled: bool,
}

impl Command {
    pub const ALL: [Self; 10] = [
        Self::Create,
        Self::Associate,
        Self::Check,
        Self::List,
        Self::Unlist,
        Self::Disassociate,
        Self::Activate,
        Self::Deactivate,
        Self::Edit,
        Self::Refresh,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Create => "listing-create",
            Self::Associate => "listing-associate",
            Self::Check => "listing-check",
            Self::List => "listing-list",
            Self::Unlist => "listing-unlist",
            Self::Disassociate => "listing-disassociate",
            Self::Activate => "listing-activate",
            Self::Deactivate => "listing-deactivate",
            Self::Edit => "listing-edit",
            Self::Refresh => "listing-refresh",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => aurora_i18n::tr!("marketplace-action-create"),
            Self::Associate => aurora_i18n::tr!("marketplace-action-associate"),
            Self::Check => aurora_i18n::tr!("marketplace-action-check"),
            Self::List => aurora_i18n::tr!("marketplace-action-list"),
            Self::Unlist => aurora_i18n::tr!("marketplace-action-unlist"),
            Self::Disassociate => aurora_i18n::tr!("marketplace-action-disassociate"),
            Self::Activate => aurora_i18n::tr!("marketplace-action-activate"),
            Self::Deactivate => aurora_i18n::tr!("marketplace-action-deactivate"),
            Self::Edit => aurora_i18n::tr!("marketplace-action-edit"),
            Self::Refresh => aurora_i18n::tr!("inventory-listing-refresh"),
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Create => "tag-filled",
            Self::Associate => "link",
            Self::Check => "clipboard-text-filled",
            Self::List | Self::Activate => "circle-check-filled",
            Self::Unlist => "eye-off",
            Self::Disassociate => "unlink",
            Self::Deactivate => "x",
            Self::Edit => "edit-filled",
            Self::Refresh => "refresh",
        }
    }
}

impl ListingsUi {
    /// LLFolderBridge::pasteFromClipboard: validate again after confirmation,
    /// since stock, permissions or the clipboard may have changed meanwhile.
    #[allow(clippy::too_many_arguments)]
    pub fn paste_inventory(
        &mut self,
        inv: &Inventory,
        worn: &HashSet<Uuid>,
        protected: &HashSet<Uuid>,
        to: Uuid,
        ids: &[Uuid],
        copy: bool,
        confirmed: bool,
        actions: &mut Vec<Action>,
    ) {
        let Some(root) = mp::root(inv) else { return };
        if !self.data.is_merchant()
            || self.data.fetch() != Fetch::Done
            || self.data.is_updating(inv, root, to)
            || ids.iter().any(|id| self.data.is_updating(inv, root, *id))
        {
            return;
        }
        let change = match crate::world::inventory::actions::paste(inv, ids, to, !copy, false, protected, worn) {
            Ok(change) => change,
            Err(reason) => {
                self.message = reason.clone();
                actions.push(notify(&reason));
                return;
            }
        };
        let active = in_active_folder(&self.data, inv, root, to) || (!copy && ids.iter().any(|id| self.affects_active(inv, root, *id)));
        if !confirmed && (active || ids.iter().any(|id| mp::depth(inv, root, *id) < 0)) {
            self.open = true;
            self.confirm = Some(Confirm::Paste {
                to,
                ids: ids.to_vec(),
                copy,
                active,
            });
        } else {
            actions.push(Action::Paste { change, cut: !copy });
        }
    }

    fn affects_active(&self, inv: &Inventory, root: Uuid, id: Uuid) -> bool {
        in_active_folder(&self.data, inv, root, id) || (self.data.is_listed(id) && self.data.activation_state(id))
    }

    /// Confirm structural edits to an active listing from any inventory panel,
    /// including Delete, Move to default folder and Create folder from selection.
    pub fn guard_inventory_change(&mut self, inv: &Inventory, change: Mutation, cut: bool) -> Option<Mutation> {
        let Some(root) = mp::root(inv) else { return Some(change) };
        if change.operations.iter().any(|operation| {
            let id = match operation {
                Operation::Folder { parent, .. } | Operation::Links { parent, .. } => *parent,
                Operation::Patch { id, .. } | Operation::Delete { id, .. } | Operation::Purge(id) => *id,
            };
            self.data.is_updating(inv, root, id)
        }) {
            return None;
        }
        let mut affected = Vec::new();
        for operation in &change.operations {
            match operation {
                Operation::Folder { parent, .. } => affected.push(*parent),
                Operation::Patch { id, body, .. } if body.has("parent_id") => {
                    affected.extend([*id, body["parent_id"].as_uuid()]);
                }
                Operation::Delete { id, .. } | Operation::Purge(id) => affected.push(*id),
                _ => {}
            }
        }
        affected.extend(change.copies.iter().map(|copy| copy.parent));
        if affected.iter().any(|id| self.data.is_updating(inv, root, *id)) {
            return None;
        }
        if affected.iter().any(|id| self.affects_active(inv, root, *id)) {
            self.open = true;
            self.confirm = Some(Confirm::Inventory { change, cut });
            None
        } else {
            Some(change)
        }
    }

    pub fn inventory_commands(&self, inv: &Inventory, root: Uuid, id: Uuid) -> Vec<OptionState> {
        let depth = mp::depth(inv, root, id);
        if depth < 1 || root.is_nil() {
            return Vec::new();
        }
        let updating = self.data.is_updating(inv, root, id);
        let mut options = Vec::new();
        let loaded = self.data.is_merchant() && self.data.fetch() == Fetch::Done;
        let mut add = |command, enabled| {
            options.push(OptionState {
                command,
                enabled: enabled && !updating && loaded,
            })
        };
        if depth == 1 && inv.folders.contains_key(&id) {
            let listed = self.data.is_listed(id);
            let version = self.data.version_folder(id);
            let live = self.data.activation_state(id);
            add(Command::Create, !listed);
            add(Command::Associate, !listed);
            add(Command::Check, true);
            add(Command::List, listed && !version.is_nil() && !live);
            add(Command::Unlist, listed && !version.is_nil() && live);
            add(Command::Disassociate, listed);
            add(Command::Refresh, listed);
        }
        if depth == 2 && inv.folders.contains_key(&id) {
            let parent = inv.folders.get(&id).map_or(Uuid::nil(), |folder| folder.info.parent);
            if self.data.is_listed(parent) {
                add(Command::Activate, !self.data.is_version_folder(id));
                add(
                    Command::Deactivate,
                    self.data.is_version_folder(id) && !self.data.activation_state(id),
                );
            }
        }
        let listing = mp::nested_parent(inv, id, depth);
        add(
            Command::Edit,
            self.data.is_listed(listing) && !self.data.version_folder(listing).is_nil(),
        );
        options
    }

    pub fn inventory_command(
        &mut self,
        inv: &Inventory,
        worn: &HashSet<Uuid>,
        root: Uuid,
        id: Uuid,
        command: Command,
        actions: &mut Vec<Action>,
    ) {
        if !self
            .inventory_commands(inv, root, id)
            .iter()
            .any(|option| option.command == command && option.enabled)
        {
            return;
        }
        match command {
            Command::Create => self.create_listing(inv, worn, root, id, actions),
            Command::Associate => {
                self.open = true;
                self.associate = Some((id, String::new()));
            }
            Command::Check => {
                self.open = true;
                self.run_audit(inv, worn, root, id);
            }
            Command::List => self.list(inv, worn, root, id, actions),
            Command::Unlist => {
                self.data.activate_listing(inv, root, id, false);
            }
            Command::Disassociate => {
                self.data.clear_listing(inv, root, id);
            }
            Command::Activate => self.activate(inv, worn, root, id, actions),
            Command::Deactivate => {
                if let Some(parent) = inv.folders.get(&id).map(|folder| folder.info.parent) {
                    self.data.set_version_folder(inv, root, parent, Uuid::nil());
                }
            }
            Command::Edit => {
                let url = self.data.listing_url(inv, root, id);
                if !url.is_empty() {
                    actions.push(Action::OpenMarketplace(url));
                }
            }
            Command::Refresh => {
                self.data.refresh_listing(inv, root, id);
            }
        }
    }
}
