use aurora_extension_api::{API_VERSION, ExtensionAction, ExtensionEvent, ExtensionRequest, PanelControl, PanelEventValue, Permission};
use aurora_extension_runtime::{
    ExtensionReview, ExtensionState, ExtensionSummary, MAX_PACKAGE_BYTES, MAX_SIGNATURE_BYTES, PersistentExtensionManager,
    PublisherTrustStatus, StagedExtension, parse_public_key_file,
};
use aurora_i18n::tr;
use crossbeam_channel::{Sender, bounded};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fmt::Write as _,
    fs,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
};

const ACTION_BATCH_LIMIT: usize = 128;
const REGISTERED_PANEL_LIMIT: usize = 256;

/// Safe, UI-ready metadata for a package awaiting installation consent.
#[derive(Clone, Debug)]
pub struct Review {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher_claim: String,
    pub content_sha256: String,
    pub requested_permissions: Vec<String>,
    pub publisher_trust: PublisherTrustStatus,
    pub added_permissions: Vec<String>,
    pub permissions_changed: bool,
    pub publisher_identity_changed: bool,
    pub previous_signature_key: Option<String>,
    pub is_update: bool,
}

/// Public metadata for one locally trusted publisher key.
#[derive(Clone, Debug)]
pub struct TrustedPublisherView {
    pub key_id: String,
    pub label: String,
}

/// Public fingerprint shown before the user accepts a publisher key.
#[derive(Clone, Debug)]
pub struct PublisherKeyReview {
    pub key_id: String,
    pub label: String,
}

/// A host-rendered, non-executable panel registered by an enabled extension.
#[derive(Clone, Debug)]
pub struct PanelView {
    pub extension_id: String,
    pub panel_id: String,
    pub title: String,
    pub controls: Vec<PanelControl>,
}

/// Validated, bounded actions returned by one guest event.
#[derive(Clone, Debug)]
pub struct ActionBatch {
    pub extension_id: String,
    pub message_id: Option<u64>,
    pub actions: Vec<ExtensionAction>,
}

/// A snapshot of extension state for the Preferences manager page.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub ready: bool,
    /// Opaque internal store scope; never passed to guest code or rendered.
    pub account_scope: Option<String>,
    pub error: Option<String>,
    pub status: Option<String>,
    pub safe_mode: bool,
    pub installed: Vec<ExtensionSummary>,
    pub trusted_publishers: Vec<TrustedPublisherView>,
    pub pending_publisher: Option<PublisherKeyReview>,
    pub pending: Option<Review>,
    pub panels: Vec<PanelView>,
}

/// User-initiated lifecycle commands issued by the extension manager UI.
pub enum Command {
    /// Select the opaque per-account (and per-grid) store before handling UI/events.
    SetAccountScope(String),
    ChooseInstall(PathBuf),
    ChooseUpdate {
        id: String,
        path: PathBuf,
    },
    ChoosePublisherKey(PathBuf),
    CommitPublisherKey,
    CancelPublisherKey,
    UntrustPublisher(String),
    CommitInstall {
        permissions: Vec<String>,
        enable: bool,
        acknowledge_unverified: bool,
        acknowledge_publisher_change: bool,
    },
    CancelInstall,
    Enable {
        id: String,
        permissions: Vec<String>,
        acknowledge_unverified: bool,
    },
    Disable {
        id: String,
    },
    Revoke {
        id: String,
        permission: String,
    },
    Uninstall {
        id: String,
    },
    ClearQuarantine {
        id: String,
    },
    SafeMode(bool),
    NearbyChat(ExtensionEvent),
    PanelAction {
        id: String,
        panel_id: String,
        control_id: String,
        value: PanelEventValue,
    },
}

enum WorkerCommand {
    SetAccountScope(String),
    /// Reject queued UI/events whose originating account no longer matches the worker.
    Scoped {
        account_scope: String,
        command: Command,
    },
}

/// Non-blocking front end to the extension supervisor thread.
pub struct Service {
    tx: Sender<WorkerCommand>,
    snapshot: Arc<RwLock<Snapshot>>,
    actions: Arc<RwLock<VecDeque<ActionBatch>>>,
}

struct PendingInstall {
    staged: StagedExtension,
    bytes: Vec<u8>,
    update_id: Option<String>,
    review: Review,
}

struct PendingPublisher {
    key_file_bytes: Vec<u8>,
    review: PublisherKeyReview,
}

impl Service {
    /// Start the isolated worker at a per-user root. Each active grid/account
    /// receives `root/scopes/<opaque-key>`; legacy root-level data is untouched.
    pub fn new(root: PathBuf) -> Self {
        let (tx, rx) = bounded(64);
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let worker_snapshot = Arc::clone(&snapshot);
        let actions = Arc::new(RwLock::new(VecDeque::new()));
        let worker_actions = Arc::clone(&actions);
        if let Err(error) = thread::Builder::new()
            .name("aurora-extension-manager".into())
            .spawn(move || run_worker(root, rx, worker_snapshot, worker_actions))
            && let Ok(mut state) = snapshot.write()
        {
            state.error = Some(tr!("extensions-manager-error-start", error = error.to_string()));
        }
        Self { tx, snapshot, actions }
    }

    /// Queue one user command without blocking the UI thread.
    pub fn submit(&self, command: Command) -> bool {
        match command {
            Command::SetAccountScope(scope) => self.enqueue(WorkerCommand::SetAccountScope(scope), true),
            command => {
                let Some(scope) = self.snapshot().account_scope else {
                    self.report_error(tr!("extensions-manager-storage-unavailable").to_owned());
                    return false;
                };
                self.submit_for_scope(&scope, command)
            }
        }
    }

    /// Queue an event or UI action with the account scope that produced it.
    pub fn submit_for_scope(&self, scope: &str, command: Command) -> bool {
        if !valid_account_scope(scope) || matches!(&command, Command::SetAccountScope(_)) {
            self.report_error(tr!("extensions-manager-storage-unavailable").to_owned());
            return false;
        }
        self.enqueue(
            WorkerCommand::Scoped {
                account_scope: scope.to_owned(),
                command,
            },
            false,
        )
    }

    fn enqueue(&self, command: WorkerCommand, scope_change: bool) -> bool {
        match self.tx.try_send(command) {
            Ok(()) => {
                if scope_change && let Ok(mut actions) = self.actions.write() {
                    actions.clear();
                }
                true
            }
            Err(_) => {
                self.report_error(tr!("extensions-manager-busy").to_owned());
                false
            }
        }
    }

    /// Surface host-side failures in Preferences › Extensions.
    pub fn report_error(&self, error: String) {
        publish_error(&self.snapshot, error);
    }

    /// Read the latest safe metadata snapshot without waiting on guest execution.
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.read().map(|state| state.clone()).unwrap_or_default()
    }

    /// Hide prior-account data until the worker has loaded the requested scope.
    pub fn snapshot_for_scope(&self, scope: &str) -> Snapshot {
        let snapshot = self.snapshot();
        if snapshot.account_scope.as_deref() == Some(scope) || (snapshot.account_scope.is_none() && snapshot.error.is_some()) {
            snapshot
        } else {
            Snapshot::default()
        }
    }

    /// Drain validated guest actions for host-side application on the UI thread.
    pub fn take_actions(&self) -> Vec<ActionBatch> {
        match self.actions.write() {
            Ok(mut actions) => actions.drain(..).collect(),
            Err(_) => {
                self.report_error(tr!("extensions-manager-actions-unavailable").to_owned());
                Vec::new()
            }
        }
    }

    /// Drain guest effects only after the worker has switched to this account.
    pub fn take_actions_for_scope(&self, scope: &str) -> Vec<ActionBatch> {
        if self.snapshot().account_scope.as_deref() != Some(scope) {
            if let Ok(mut actions) = self.actions.write() {
                actions.clear();
            }
            return Vec::new();
        }
        self.take_actions()
    }
}

/// Derive an opaque, stable extension-store key from the active grid and agent.
/// The raw account identifier and login URI are never stored in the path.
pub(crate) fn account_scope_key(login_uri: &str, account_id: Option<uuid::Uuid>) -> String {
    let Some(account_id) = account_id else {
        return "unscoped".to_owned();
    };
    let mut digest = Sha256::new();
    digest.update(b"aurora-extension-account-scope-v1\0");
    digest.update((login_uri.len() as u64).to_le_bytes());
    digest.update(login_uri.as_bytes());
    digest.update(account_id.as_bytes());
    let mut scope = String::from("account-");
    for byte in digest.finalize() {
        let _ = write!(&mut scope, "{byte:02x}");
    }
    scope
}

fn valid_account_scope(scope: &str) -> bool {
    scope == "unscoped"
        || scope
            .strip_prefix("account-")
            .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

fn run_worker(
    root: PathBuf,
    rx: crossbeam_channel::Receiver<WorkerCommand>,
    shared: Arc<RwLock<Snapshot>>,
    shared_actions: Arc<RwLock<VecDeque<ActionBatch>>>,
) {
    let mut manager: Option<PersistentExtensionManager> = None;
    let mut active_scope: Option<String> = None;
    let mut pending: Option<PendingInstall> = None;
    let mut pending_publisher: Option<PendingPublisher> = None;
    let mut panels = Vec::new();
    let mut next_request_id = 1u64;
    publish_state(&shared, None, &panels, None);

    for message in rx {
        let command = match message {
            WorkerCommand::Scoped { account_scope, command } => {
                if active_scope.as_deref() != Some(account_scope.as_str()) {
                    continue;
                }
                command
            }
            WorkerCommand::SetAccountScope(scope) => {
                if !valid_account_scope(&scope) {
                    publish_error(&shared, tr!("extensions-manager-storage-unavailable").to_owned());
                    continue;
                }
                if active_scope.as_deref() == Some(scope.as_str()) {
                    continue;
                }
                // Dropping the old manager is sufficient: each call has fresh guest
                // memory and no persistent host handles. Do not dispatch Shutdown
                // across accounts, where it could create stale effects or quarantine state.
                manager = None;
                active_scope = Some(scope.clone());
                next_request_id = 1;
                pending = None;
                pending_publisher = None;
                panels.clear();
                update_pending(&shared, None);
                update_pending_publisher(&shared, None);
                if let Ok(mut actions) = shared_actions.write() {
                    actions.clear();
                }
                reset_snapshot_for_scope(&shared, Some(scope.clone()));
                let scoped_root = root.join("scopes").join(&scope);
                match PersistentExtensionManager::open(scoped_root, Version::new(1, 0, 0)) {
                    Ok(mut opened) => {
                        let enabled: Vec<_> = opened
                            .list()
                            .into_iter()
                            .filter(|extension| extension.state == ExtensionState::Enabled)
                            .map(|extension| extension.manifest.id)
                            .collect();
                        let mut errors = Vec::new();
                        for id in enabled {
                            if let Err(error) = initialize_extension(&mut opened, &id, &mut next_request_id, &shared_actions, &mut panels) {
                                errors.push(error);
                            }
                        }
                        manager = Some(opened);
                        if errors.is_empty() {
                            publish_state(&shared, manager.as_ref(), &panels, None);
                        } else {
                            publish_failure(&shared, manager.as_ref(), &panels, errors.join("\n"));
                        }
                    }
                    Err(error) => {
                        publish_error(&shared, tr!("extensions-manager-error-load", error = error.to_string()));
                    }
                }
                continue;
            }
        };
        let Some(manager) = manager.as_mut() else {
            publish_error(&shared, tr!("extensions-manager-storage-unavailable").to_owned());
            continue;
        };
        let command = match command {
            Command::NearbyChat(event) => {
                let mut errors = Vec::new();
                if matches!(&event, ExtensionEvent::NearbyChat { .. }) && !manager.safe_mode() {
                    let eligible: Vec<_> = manager
                        .list()
                        .into_iter()
                        .filter(|extension| {
                            extension.state == ExtensionState::Enabled
                                && extension
                                    .granted_permissions
                                    .iter()
                                    .any(|grant| grant.covers(Permission::ChatNearbyRead))
                        })
                        .map(|extension| extension.manifest.id)
                        .collect();
                    for id in eligible {
                        let request = make_request(&mut next_request_id, event.clone());
                        if let Err(error) = dispatch_extension(manager, &id, request, &shared_actions, &mut panels) {
                            errors.push(error);
                        }
                    }
                }
                if errors.is_empty() {
                    publish_state(&shared, Some(manager), &panels, None);
                } else {
                    publish_failure(&shared, Some(manager), &panels, errors.join("\n"));
                }
                continue;
            }
            Command::PanelAction {
                id,
                panel_id,
                control_id,
                value,
            } => {
                let result = if manager.safe_mode() {
                    Ok(())
                } else if panel_has_control(&panels, &id, &panel_id, &control_id) {
                    update_toggle_value(&mut panels, &id, &panel_id, &control_id, &value);
                    let request = make_request(
                        &mut next_request_id,
                        ExtensionEvent::PanelAction {
                            panel_id,
                            control_id,
                            value,
                        },
                    );
                    dispatch_extension(manager, &id, request, &shared_actions, &mut panels)
                } else {
                    Err(tr!("extensions-manager-panel-action-stale", id = id.as_str()))
                };
                match result {
                    Ok(()) => publish_state(&shared, Some(manager), &panels, None),
                    Err(error) => publish_failure(&shared, Some(manager), &panels, error),
                }
                continue;
            }
            other => other,
        };

        let result = match command {
            Command::ChooseInstall(path) => {
                if pending.is_some() || pending_publisher.is_some() {
                    Err(tr!("extensions-manager-finish-review").to_owned())
                } else {
                    stage_file(manager, &path, None).map(|candidate| {
                        update_pending(&shared, Some(candidate.review.clone()));
                        pending = Some(candidate);
                        tr!("extensions-manager-package-validated").to_owned()
                    })
                }
            }
            Command::ChooseUpdate { id, path } => {
                if pending.is_some() || pending_publisher.is_some() {
                    Err(tr!("extensions-manager-finish-review").to_owned())
                } else {
                    stage_file(manager, &path, Some(id)).map(|candidate| {
                        update_pending(&shared, Some(candidate.review.clone()));
                        pending = Some(candidate);
                        tr!("extensions-manager-update-validated").to_owned()
                    })
                }
            }
            Command::ChoosePublisherKey(path) => {
                if pending.is_some() || pending_publisher.is_some() {
                    Err(tr!("extensions-manager-finish-review").to_owned())
                } else {
                    stage_publisher_key(&path).map(|candidate| {
                        update_pending_publisher(&shared, Some(candidate.review.clone()));
                        pending_publisher = Some(candidate);
                        tr!("extensions-manager-publisher-key-reviewed").to_owned()
                    })
                }
            }
            Command::CommitPublisherKey => {
                let Some(candidate) = pending_publisher.take() else {
                    publish_error(&shared, tr!("extensions-manager-no-pending-publisher").to_owned());
                    continue;
                };
                match manager.trust_publisher(&candidate.key_file_bytes, &candidate.review.label) {
                    Ok(publisher) => {
                        update_pending_publisher(&shared, None);
                        Ok(tr!("extensions-manager-publisher-trusted", key = publisher.key_id.as_str()))
                    }
                    Err(error) => {
                        pending_publisher = Some(candidate);
                        Err(operation_error(error))
                    }
                }
            }
            Command::CancelPublisherKey => {
                pending_publisher = None;
                update_pending_publisher(&shared, None);
                Ok(tr!("extensions-manager-publisher-trust-cancelled").to_owned())
            }
            Command::UntrustPublisher(key_id) => {
                if pending.is_some() || pending_publisher.is_some() {
                    Err(tr!("extensions-manager-finish-review").to_owned())
                } else {
                    manager
                        .untrust_publisher(&key_id)
                        .map(|()| {
                            let active: std::collections::HashSet<_> = manager
                                .list()
                                .into_iter()
                                .filter(|extension| extension.state == ExtensionState::Enabled)
                                .map(|extension| extension.manifest.id)
                                .collect();
                            panels.retain(|panel| active.contains(&panel.extension_id));
                            tr!("extensions-manager-publisher-untrusted", key = key_id.as_str())
                        })
                        .map_err(operation_error)
                }
            }
            Command::CommitInstall {
                permissions,
                enable,
                acknowledge_unverified,
                acknowledge_publisher_change,
            } => {
                let Some(candidate) = pending.take() else {
                    publish_error(&shared, tr!("extensions-manager-no-pending").to_owned());
                    continue;
                };
                let PendingInstall {
                    staged,
                    bytes,
                    update_id,
                    review,
                } = candidate;
                match parse_permissions(&permissions) {
                    Ok(approved) => {
                        let result = if let Some(id) = update_id.as_deref() {
                            manager.update_with_ack(
                                id,
                                staged,
                                &bytes,
                                &approved,
                                enable,
                                acknowledge_unverified,
                                acknowledge_publisher_change,
                            )
                        } else {
                            manager.install_with_ack(
                                staged,
                                &bytes,
                                &approved,
                                enable,
                                acknowledge_unverified,
                                acknowledge_publisher_change,
                            )
                        };
                        match result {
                            Ok(summary) => {
                                let initialized = if enable {
                                    initialize_extension(manager, &summary.manifest.id, &mut next_request_id, &shared_actions, &mut panels)
                                } else {
                                    panels.retain(|panel| panel.extension_id != summary.manifest.id);
                                    Ok(())
                                };
                                update_pending(&shared, None);
                                if let Err(error) = initialized {
                                    Err(error)
                                } else if review.is_update {
                                    Ok(tr!(
                                        "extensions-manager-updated-status",
                                        name = summary.manifest.name.as_str(),
                                        version = summary.manifest.version.as_str()
                                    ))
                                } else {
                                    Ok(tr!(
                                        "extensions-manager-installed-status",
                                        name = summary.manifest.name.as_str(),
                                        version = summary.manifest.version.as_str()
                                    ))
                                }
                            }
                            Err(error) => {
                                let retry = if let Some(id) = update_id.as_deref() {
                                    manager.stage_update(id, &bytes)
                                } else {
                                    manager.stage_install(&bytes)
                                };
                                pending = retry.ok().map(|staged| PendingInstall {
                                    staged,
                                    bytes,
                                    update_id,
                                    review,
                                });
                                Err(operation_error(error))
                            }
                        }
                    }
                    Err(error) => {
                        pending = Some(PendingInstall {
                            staged,
                            bytes,
                            update_id,
                            review,
                        });
                        Err(error)
                    }
                }
            }
            Command::CancelInstall => {
                pending = None;
                update_pending(&shared, None);
                Ok(tr!("extensions-manager-install-cancelled").to_owned())
            }
            Command::Enable {
                id,
                permissions,
                acknowledge_unverified,
            } => match parse_permissions(&permissions) {
                Ok(grants) => match manager.enable_with_ack(&id, &grants, acknowledge_unverified) {
                    Ok(()) => initialize_extension(manager, &id, &mut next_request_id, &shared_actions, &mut panels)
                        .map(|()| tr!("extensions-manager-enabled-status", id = id.as_str())),
                    Err(error) => Err(operation_error(error)),
                },
                Err(error) => Err(error),
            },
            Command::Disable { id } => manager
                .disable(&id)
                .map(|()| {
                    panels.retain(|panel| panel.extension_id != id);
                    tr!("extensions-manager-disabled-status", id = id.as_str())
                })
                .map_err(operation_error),
            Command::Revoke { id, permission } => Permission::from_manifest_key(&permission)
                .ok_or_else(|| tr!("extensions-manager-unknown-permission", permission = permission.as_str()))
                .and_then(|permission| {
                    manager.revoke_permission(&id, permission).map_err(operation_error)?;
                    panels.retain(|panel| panel.extension_id != id);
                    Ok(tr!(
                        "extensions-manager-revoked-status",
                        permission = permission.manifest_key(),
                        id = id.as_str()
                    ))
                }),
            Command::Uninstall { id } => manager
                .uninstall(&id)
                .map(|()| {
                    panels.retain(|panel| panel.extension_id != id);
                    tr!("extensions-manager-uninstalled-status", id = id.as_str())
                })
                .map_err(operation_error),
            Command::ClearQuarantine { id } => manager
                .clear_quarantine(&id)
                .map(|()| {
                    panels.retain(|panel| panel.extension_id != id);
                    tr!("extensions-manager-quarantine-cleared-status", id = id.as_str())
                })
                .map_err(operation_error),
            Command::SafeMode(enabled) => manager
                .set_safe_mode(enabled)
                .map(|()| {
                    let mut errors = Vec::new();
                    if enabled {
                        panels.clear();
                    } else {
                        let enabled_ids: Vec<_> = manager
                            .list()
                            .into_iter()
                            .filter(|extension| extension.state == ExtensionState::Enabled)
                            .map(|extension| extension.manifest.id)
                            .collect();
                        for id in enabled_ids {
                            if let Err(error) = initialize_extension(manager, &id, &mut next_request_id, &shared_actions, &mut panels) {
                                errors.push(error);
                            }
                        }
                    }
                    if !errors.is_empty() {
                        Err(errors.join("\n"))
                    } else if enabled {
                        Ok(tr!("extensions-manager-safe-mode-on").to_owned())
                    } else {
                        Ok(tr!("extensions-manager-safe-mode-off").to_owned())
                    }
                })
                .map_err(operation_error)
                .and_then(|result| result),
            Command::SetAccountScope(_) | Command::NearbyChat(_) | Command::PanelAction { .. } => {
                unreachable!("worker event handled above")
            }
        };
        match result {
            Ok(status) => publish_state(&shared, Some(manager), &panels, Some(status)),
            Err(error) => publish_failure(&shared, Some(manager), &panels, error),
        }
    }
    if let Some(manager) = manager.as_mut()
        && !manager.safe_mode()
    {
        let enabled: Vec<_> = manager
            .list()
            .into_iter()
            .filter(|extension| extension.state == ExtensionState::Enabled)
            .map(|extension| extension.manifest.id)
            .collect();
        for id in enabled {
            let request = make_request(&mut next_request_id, ExtensionEvent::Shutdown);
            let _ = manager.dispatch(&id, &request);
        }
    }
}

fn make_request(next_request_id: &mut u64, event: ExtensionEvent) -> ExtensionRequest {
    let request_id = *next_request_id;
    *next_request_id = next_request_id.wrapping_add(1).max(1);
    ExtensionRequest {
        api_version: API_VERSION,
        request_id,
        event,
        storage: None,
    }
}

fn initialize_extension(
    manager: &mut PersistentExtensionManager,
    id: &str,
    next_request_id: &mut u64,
    actions: &RwLock<VecDeque<ActionBatch>>,
    panels: &mut Vec<PanelView>,
) -> Result<(), String> {
    panels.retain(|panel| panel.extension_id != id);
    let request = make_request(
        next_request_id,
        ExtensionEvent::Initialize {
            host_api_version: "1.0.0".into(),
        },
    );
    dispatch_extension(manager, id, request, actions, panels)
}

fn dispatch_extension(
    manager: &mut PersistentExtensionManager,
    id: &str,
    mut request: ExtensionRequest,
    action_queue: &RwLock<VecDeque<ActionBatch>>,
    panels: &mut Vec<PanelView>,
) -> Result<(), String> {
    request.storage = manager.storage_snapshot(id).map_err(|error| error.to_string())?;
    let message_id = match &request.event {
        ExtensionEvent::NearbyChat { message_id, .. } => Some(*message_id),
        _ => None,
    };
    match manager.dispatch(id, &request) {
        Ok(actions) => {
            let new_panels = actions
                .iter()
                .filter_map(|action| match action {
                    ExtensionAction::RegisterPanel { panel_id, .. }
                        if !panels.iter().any(|panel| panel.extension_id == id && panel.panel_id == *panel_id) =>
                    {
                        Some(panel_id)
                    }
                    _ => None,
                })
                .count();
            if panels.len().saturating_add(new_panels) > REGISTERED_PANEL_LIMIT {
                return Err(tr!("extensions-manager-panels-full").to_owned());
            }
            let mut queue = if actions.is_empty() {
                None
            } else {
                let queue = action_queue
                    .write()
                    .map_err(|_| tr!("extensions-manager-actions-unavailable").to_owned())?;
                if queue.len() >= ACTION_BATCH_LIMIT {
                    return Err(tr!("extensions-manager-actions-full").to_owned());
                }
                Some(queue)
            };
            for action in &actions {
                if let ExtensionAction::RegisterPanel { panel_id, title, controls } = action {
                    if let Some(panel) = panels
                        .iter_mut()
                        .find(|panel| panel.extension_id == id && panel.panel_id == *panel_id)
                    {
                        panel.title.clone_from(title);
                        panel.controls.clone_from(controls);
                    } else {
                        panels.push(PanelView {
                            extension_id: id.to_owned(),
                            panel_id: panel_id.clone(),
                            title: title.clone(),
                            controls: controls.clone(),
                        });
                    }
                }
            }
            if let Some(queue) = queue.as_mut() {
                queue.push_back(ActionBatch {
                    extension_id: id.to_owned(),
                    message_id,
                    actions,
                });
            }
            Ok(())
        }
        Err(error) => {
            if manager
                .list()
                .iter()
                .any(|summary| summary.manifest.id == id && summary.state == ExtensionState::Quarantined)
            {
                panels.retain(|panel| panel.extension_id != id);
            }
            Err(tr!("extensions-manager-dispatch-failed", id = id, reason = error.to_string()))
        }
    }
}

/// Route a validated guest request into Aurora's host-rendered notification center.
/// The guest supplies plain text only; the viewer owns the card, style, timing and actions.
pub(super) fn show_notification(notifications: &mut crate::world::notifications::Notifications, title: String, body: String) -> u64 {
    notifications.push(
        crate::world::notifications::Kind::Info,
        title,
        body,
        crate::world::notifications::Data::None,
    )
}

fn panel_has_control(panels: &[PanelView], extension: &str, panel_id: &str, control_id: &str) -> bool {
    panels.iter().any(|panel| {
        panel.extension_id == extension
            && panel.panel_id == panel_id
            && panel.controls.iter().any(|control| match control {
                PanelControl::Label { id, .. } | PanelControl::Toggle { id, .. } | PanelControl::Button { id, .. } => id == control_id,
            })
    })
}

fn update_toggle_value(panels: &mut [PanelView], extension: &str, panel_id: &str, control_id: &str, value: &PanelEventValue) {
    let PanelEventValue::Toggled { value } = value else {
        return;
    };
    if let Some(PanelControl::Toggle { value: current, .. }) = panels
        .iter_mut()
        .find(|panel| panel.extension_id == extension && panel.panel_id == panel_id)
        .and_then(|panel| {
            panel
                .controls
                .iter_mut()
                .find(|control| matches!(control, PanelControl::Toggle { id, .. } if id == control_id))
        })
    {
        *current = *value;
    }
}

fn stage_file(manager: &PersistentExtensionManager, path: &PathBuf, update_id: Option<String>) -> Result<PendingInstall, String> {
    let metadata = fs::metadata(path).map_err(|error| tr!("extensions-manager-package-read-error", error = error.to_string()))?;
    if metadata.len() > MAX_PACKAGE_BYTES as u64 {
        return Err(tr!("extensions-manager-package-too-large").to_owned());
    }
    let bytes = fs::read(path).map_err(|error| tr!("extensions-manager-package-read-error", error = error.to_string()))?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(tr!("extensions-manager-package-too-large").to_owned());
    }
    let staged = if let Some(id) = update_id.as_deref() {
        manager.stage_update(id, &bytes)
    } else {
        manager.stage_install(&bytes)
    }
    .map_err(|error| {
        if let Some(reason) = error.blocked_reason() {
            tr!("extensions-manager-blocked-plugin", reason = reason)
        } else {
            tr!("extensions-manager-package-invalid", error = error.to_string())
        }
    })?;
    let review = review_view(staged.review(), update_id.is_some());
    Ok(PendingInstall {
        staged,
        bytes,
        update_id,
        review,
    })
}

fn stage_publisher_key(path: &PathBuf) -> Result<PendingPublisher, String> {
    let metadata = fs::metadata(path).map_err(|error| tr!("extensions-manager-publisher-key-read-error", error = error.to_string()))?;
    if metadata.len() > MAX_SIGNATURE_BYTES as u64 {
        return Err(tr!("extensions-manager-publisher-key-too-large").to_owned());
    }
    let key_file_bytes = fs::read(path).map_err(|error| tr!("extensions-manager-publisher-key-read-error", error = error.to_string()))?;
    if key_file_bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(tr!("extensions-manager-publisher-key-too-large").to_owned());
    }
    let verified = parse_public_key_file(&key_file_bytes)
        .map_err(|error| tr!("extensions-manager-publisher-key-invalid", error = error.to_string()))?;
    let label = format!("Key {}", &verified.key_id[..12]);
    Ok(PendingPublisher {
        key_file_bytes,
        review: PublisherKeyReview {
            key_id: verified.key_id,
            label,
        },
    })
}

fn review_view(review: &ExtensionReview, is_update: bool) -> Review {
    Review {
        id: review.manifest.id.clone(),
        name: review.manifest.name.clone(),
        version: review.manifest.version.clone(),
        publisher_claim: review.manifest.publisher.clone(),
        content_sha256: review.content_sha256.iter().map(|byte| format!("{byte:02x}")).collect(),
        requested_permissions: review
            .requested_permissions
            .iter()
            .map(|permission| permission.manifest_key().to_owned())
            .collect(),
        publisher_trust: review.publisher_trust.clone(),
        added_permissions: review
            .added_permissions
            .iter()
            .map(|permission| permission.manifest_key().to_owned())
            .collect(),
        permissions_changed: review.permissions_changed,
        publisher_identity_changed: review.publisher_identity_changed,
        previous_signature_key: review.previous_signature_key.clone(),
        is_update,
    }
}

fn parse_permissions(keys: &[String]) -> Result<Vec<Permission>, String> {
    let mut seen = std::collections::HashSet::new();
    keys.iter()
        .map(|key| {
            let permission = Permission::from_manifest_key(key)
                .ok_or_else(|| tr!("extensions-manager-unknown-permission", permission = key.as_str()))?;
            if !seen.insert(permission) {
                return Err(tr!("extensions-manager-duplicate-permission", permission = key.as_str()));
            }
            Ok(permission)
        })
        .collect()
}

fn operation_error(error: impl std::fmt::Display) -> String {
    tr!("extensions-manager-operation-error", error = error.to_string())
}

fn publish_state(shared: &RwLock<Snapshot>, manager: Option<&PersistentExtensionManager>, panels: &[PanelView], status: Option<String>) {
    if let Ok(mut snapshot) = shared.write() {
        snapshot.ready = manager.is_some();
        if let Some(manager) = manager {
            snapshot.error = None;
            snapshot.safe_mode = manager.safe_mode();
            snapshot.installed = manager.list();
            snapshot.trusted_publishers = manager
                .trusted_publishers()
                .iter()
                .map(|publisher| TrustedPublisherView {
                    key_id: publisher.key_id.clone(),
                    label: publisher.label.clone(),
                })
                .collect();
            snapshot.panels = panels.to_vec();
        }
        snapshot.status = status;
    }
}

fn publish_failure(shared: &RwLock<Snapshot>, manager: Option<&PersistentExtensionManager>, panels: &[PanelView], error: String) {
    if let Ok(mut snapshot) = shared.write() {
        snapshot.ready = manager.is_some();
        if let Some(manager) = manager {
            snapshot.safe_mode = manager.safe_mode();
            snapshot.installed = manager.list();
            snapshot.trusted_publishers = manager
                .trusted_publishers()
                .iter()
                .map(|publisher| TrustedPublisherView {
                    key_id: publisher.key_id.clone(),
                    label: publisher.label.clone(),
                })
                .collect();
            snapshot.panels = panels.to_vec();
        }
        snapshot.error = Some(error);
        snapshot.status = None;
    }
}

fn publish_error(shared: &RwLock<Snapshot>, error: String) {
    if let Ok(mut snapshot) = shared.write() {
        snapshot.error = Some(error);
        snapshot.status = None;
    }
}

fn reset_snapshot_for_scope(shared: &RwLock<Snapshot>, account_scope: Option<String>) {
    if let Ok(mut snapshot) = shared.write() {
        *snapshot = Snapshot {
            account_scope,
            ..Snapshot::default()
        };
    }
}

fn update_pending(shared: &RwLock<Snapshot>, pending: Option<Review>) {
    if let Ok(mut snapshot) = shared.write() {
        snapshot.pending = pending;
    }
}

fn update_pending_publisher(shared: &RwLock<Snapshot>, pending: Option<PublisherKeyReview>) {
    if let Ok(mut snapshot) = shared.write() {
        snapshot.pending_publisher = pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_extension_api::{API_VERSION, ExtensionResponse, Permission};
    use aurora_extension_runtime::ExtensionState;
    use std::{
        io::{Cursor, Write},
        path::{Path, PathBuf},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};

    const DEMO_ID: &str = "dev.example.sandbox-demo";
    const DEMO_TITLE: &str = "Sandbox demo";
    const DEMO_BODY: &str = "The button event was handled by the WASM guest.";

    #[test]
    fn account_scopes_are_opaque_and_isolated_by_grid_and_account() {
        let account = uuid::Uuid::from_u128(0x123456789abcdef0123456789abcdef0);
        let scope = account_scope_key("https://grid-a.example/login", Some(account));
        assert_eq!(scope, account_scope_key("https://grid-a.example/login", Some(account)));
        assert_ne!(scope, account_scope_key("https://grid-b.example/login", Some(account)));
        assert_ne!(
            scope,
            account_scope_key("https://grid-a.example/login", Some(uuid::Uuid::from_u128(1)))
        );
        assert!(!scope.contains(&account.to_string()));
        assert!(valid_account_scope(&scope));
        assert_eq!(account_scope_key("", None), "unscoped");
        assert!(valid_account_scope("unscoped"));
        assert!(!valid_account_scope("../../other-account"));
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after epoch").as_nanos();
            Self(std::env::temp_dir().join(format!("aurora-{label}-{}-{stamp}", std::process::id())))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn initialize_request(request_id: u64) -> ExtensionRequest {
        ExtensionRequest {
            api_version: API_VERSION,
            request_id,
            event: ExtensionEvent::Initialize {
                host_api_version: "1.0.0".into(),
            },
            storage: None,
        }
    }

    fn panel_request(request_id: u64) -> ExtensionRequest {
        ExtensionRequest {
            api_version: API_VERSION,
            request_id,
            event: ExtensionEvent::PanelAction {
                panel_id: "sandbox-demo".into(),
                control_id: "notify".into(),
                value: PanelEventValue::Clicked,
            },
            storage: None,
        }
    }

    fn fixture_package(permissions: &[&str]) -> Vec<u8> {
        fixture_package_with_request_ids(permissions, 41, 42)
    }

    fn fixture_package_with_request_ids(permissions: &[&str], initialize_id: u64, click_id: u64) -> Vec<u8> {
        let initialize = initialize_request(initialize_id);
        let click = panel_request(click_id);
        let initialize_json = serde_json::to_vec(&initialize).expect("initialize request");
        let click_json = serde_json::to_vec(&click).expect("panel request");
        let event_offset = initialize_json
            .windows(b"initialize".len())
            .position(|window| window == b"initialize")
            .expect("initialize event type");
        assert_eq!(
            click_json
                .windows(b"panel_action".len())
                .position(|window| window == b"panel_action"),
            Some(event_offset),
            "the fixture events keep their type at the same serialized offset"
        );

        let init_response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: initialize.request_id,
            actions: vec![ExtensionAction::RegisterPanel {
                panel_id: "sandbox-demo".into(),
                title: "Sandbox Demo".into(),
                controls: vec![
                    PanelControl::Label {
                        id: "offline-status".into(),
                        text: "Offline fixture: the WASM guest initialized.".into(),
                    },
                    PanelControl::Toggle {
                        id: "enabled".into(),
                        label: "Demo feature enabled".into(),
                        value: false,
                    },
                    PanelControl::Button {
                        id: "notify".into(),
                        label: "Show demo notification".into(),
                    },
                ],
            }],
        };
        let mut click_response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: click.request_id,
            actions: vec![
                ExtensionAction::ShowNotification {
                    title: DEMO_TITLE.into(),
                    body: DEMO_BODY.into(),
                },
                ExtensionAction::RegisterPanel {
                    panel_id: "sandbox-demo".into(),
                    title: "Sandbox Demo".into(),
                    controls: vec![
                        PanelControl::Label {
                            id: "offline-status".into(),
                            text: "Offline fixture: the WASM guest initialized.".into(),
                        },
                        PanelControl::Toggle {
                            id: "enabled".into(),
                            label: "Demo feature enabled".into(),
                            value: true,
                        },
                        PanelControl::Button {
                            id: "notify".into(),
                            label: "Show demo notification".into(),
                        },
                    ],
                },
            ],
        };
        if permissions.contains(&"extension.storage") {
            click_response.actions.push(ExtensionAction::SetStorage {
                key: "ui.enabled".into(),
                value: Some("true".into()),
            });
        }
        let init_bytes = serde_json::to_vec(&init_response).expect("initialize response");
        let click_bytes = serde_json::to_vec(&click_response).expect("click response");
        let wasm = response_module(&init_bytes, &click_bytes, event_offset as u32);

        let permissions = serde_json::to_string(permissions).expect("permissions JSON");
        let manifest = format!(
            r#"{{"manifest_version":1,"id":"{DEMO_ID}","name":"Sandbox Demo Extension","version":"0.2.0","api":">=1.0.0, <2.0.0","entry":"module.wasm","permissions":{permissions},"network_domains":[],"publisher":"Generic demo fixture"}}"#
        );
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        archive.start_file("manifest.json", options).expect("manifest member");
        archive.write_all(manifest.as_bytes()).expect("manifest bytes");
        archive.start_file("module.wasm", options).expect("module member");
        archive.write_all(&wasm).expect("module bytes");
        archive.finish().expect("package archive").into_inner()
    }

    fn response_module(initialize: &[u8], click: &[u8], event_offset: u32) -> Vec<u8> {
        let initialize_offset = 64i32;
        let click_offset = initialize_offset + initialize.len() as i32;
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        wasm_section(
            &mut module,
            1,
            &[2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7f],
        );
        wasm_section(&mut module, 3, &[2, 0, 1]);
        wasm_section(&mut module, 5, &[1, 0, 1]);
        wasm_section(&mut module, 6, &[1, 0x7f, 1, 0x41, 0x80, 0x10, 0x0b]);
        wasm_section(
            &mut module,
            7,
            &[
                3, 6, b'm', b'e', b'm', b'o', b'r', b'y', 2, 0, 5, b'a', b'l', b'l', b'o', b'c', 0, 0, 6, b'h', b'a', b'n', b'd', b'l',
                b'e', 0, 1,
            ],
        );
        let allocate = [0, 0x23, 0, 0x23, 0, 0x20, 0, 0x6a, 0x24, 0, 0x0b];
        let mut handle = vec![0, 0x20, 0, 0x41];
        push_sleb(&mut handle, event_offset as i32);
        handle.extend_from_slice(&[0x6a, 0x2d, 0, 0, 0x41]);
        push_sleb(&mut handle, i32::from(b'p'));
        handle.extend_from_slice(&[0x46, 0x04, 0x7f, 0x20, 2, 0x41]);
        push_sleb(&mut handle, click_offset);
        handle.push(0x41);
        push_sleb(&mut handle, click.len() as i32);
        handle.extend_from_slice(&[0xfc, 0x0a, 0, 0, 0x41]);
        push_sleb(&mut handle, click.len() as i32);
        handle.extend_from_slice(&[0x05, 0x20, 2, 0x41]);
        push_sleb(&mut handle, initialize_offset);
        handle.push(0x41);
        push_sleb(&mut handle, initialize.len() as i32);
        handle.extend_from_slice(&[0xfc, 0x0a, 0, 0, 0x41]);
        push_sleb(&mut handle, initialize.len() as i32);
        handle.extend_from_slice(&[0x0b, 0x0b]);

        let mut code = vec![2];
        for body in [allocate.as_slice(), handle.as_slice()] {
            push_uleb(&mut code, body.len() as u32);
            code.extend_from_slice(body);
        }
        wasm_section(&mut module, 10, &code);
        let mut data = vec![2];
        for (offset, bytes) in [(initialize_offset, initialize), (click_offset, click)] {
            data.push(0);
            data.push(0x41);
            push_sleb(&mut data, offset);
            data.push(0x0b);
            push_uleb(&mut data, bytes.len() as u32);
            data.extend_from_slice(bytes);
        }
        wasm_section(&mut module, 11, &data);
        module
    }

    fn wasm_section(module: &mut Vec<u8>, id: u8, payload: &[u8]) {
        module.push(id);
        push_uleb(module, payload.len() as u32);
        module.extend_from_slice(payload);
    }

    fn push_uleb(bytes: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn push_sleb(bytes: &mut Vec<u8>, mut value: i32) {
        loop {
            let byte = (value as u8) & 0x7f;
            value >>= 7;
            let sign = byte & 0x40 != 0;
            let more = !((value == 0 && !sign) || (value == -1 && sign));
            bytes.push(byte | if more { 0x80 } else { 0 });
            if !more {
                break;
            }
        }
    }

    fn install_fixture(root: &Path, permissions: &[Permission]) -> (PersistentExtensionManager, Vec<u8>) {
        let keys: Vec<_> = permissions.iter().map(|permission| permission.manifest_key()).collect();
        let bytes = fixture_package(&keys);
        let mut manager = PersistentExtensionManager::open(root, Version::new(1, 0, 0)).expect("open manager");
        let staged = manager.stage_install(&bytes).expect("stage package");
        manager
            .install_with_ack(staged, &bytes, permissions, true, true, false)
            .expect("install and grant fixture");
        (manager, bytes)
    }

    fn wait_for_snapshot(service: &Service, scope: &str, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = service.snapshot_for_scope(scope);
            if predicate(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for extension manager state: {snapshot:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn panel_click_runs_wasm_with_full_access_and_shows_an_in_view_card_while_active() {
        let temp = TempRoot::new("extension-notification");
        let permissions = [Permission::FullAccess];
        let (mut manager, _) = install_fixture(&temp.0, &permissions);
        let batches = RwLock::new(VecDeque::new());
        let mut panels = Vec::new();

        dispatch_extension(&mut manager, DEMO_ID, initialize_request(41), &batches, &mut panels).expect("initialize guest");
        assert!(panel_has_control(&panels, DEMO_ID, "sandbox-demo", "notify"));
        assert_eq!(panels[0].title, "Sandbox Demo");
        batches.write().expect("action queue").clear();

        dispatch_extension(&mut manager, DEMO_ID, panel_request(42), &batches, &mut panels).expect("dispatch button click to WASM");
        let enabled = panels[0].controls.iter().find_map(|control| match control {
            PanelControl::Toggle { id, value, .. } if id == "enabled" => Some(*value),
            _ => None,
        });
        assert_eq!(enabled, Some(true), "the host panel reflects the guest's updated toggle state");
        let batch = batches.write().expect("action queue").pop_front().expect("guest response batch");
        assert_eq!(batch.extension_id, DEMO_ID);
        assert_eq!(batch.message_id, None);

        let mut notifications = crate::world::notifications::Notifications::default();
        let mut notification_id = None;
        for action in batch.actions {
            match action {
                ExtensionAction::ShowNotification { title, body } => {
                    notification_id = Some(show_notification(&mut notifications, title, body));
                }
                ExtensionAction::RegisterPanel { .. } => {}
                other => panic!("unexpected guest action: {other:?}"),
            }
        }
        let notification_id = notification_id.expect("host accepted notification action");
        let notice = notifications
            .list
            .iter()
            .find(|notice| notice.id == notification_id)
            .expect("in-view notification");
        assert_eq!(notice.title, DEMO_TITLE);
        assert_eq!(notice.body, DEMO_BODY);
        assert_eq!(
            format!("{} — {}", notice.title, notice.body),
            "Sandbox demo — The button event was handled by the WASM guest."
        );
        assert!(matches!(notice.data, crate::world::notifications::Data::None));

        // The in-view card is independent of OS focus/system notification settings.
        let mut center = crate::ui::notification_center::CenterUi::default();
        assert!(crate::ui::notification_center::update(
            &notifications,
            &mut center,
            crate::ui::notification_center::Moment {
                now: 1.0,
                ..Default::default()
            },
            30.0
        ));
        assert_eq!(center.peek.iter().map(|peek| peek.id).collect::<Vec<_>>(), vec![notification_id]);
        assert_eq!(crate::ui::notification_center::bell_count(&notifications, &center), 0);

        // Do Not Disturb pauses the card but keeps the notice in the bell's To do list.
        let mut paused = crate::ui::notification_center::CenterUi::default();
        crate::ui::notification_center::update(
            &notifications,
            &mut paused,
            crate::ui::notification_center::Moment {
                now: 2.0,
                dnd: true,
                ..Default::default()
            },
            30.0,
        );
        assert!(paused.peek.is_empty());
        assert_eq!(crate::ui::notification_center::bell_count(&notifications, &paused), 1);
    }

    #[test]
    fn panel_action_persists_only_explicitly_granted_extension_storage() {
        let temp = TempRoot::new("extension-storage");
        let permissions = [
            Permission::UiPanelRegister,
            Permission::UiNotificationShow,
            Permission::ExtensionStorage,
        ];
        let (mut manager, _) = install_fixture(&temp.0, &permissions);
        let batches = RwLock::new(VecDeque::new());
        let mut panels = Vec::new();

        dispatch_extension(&mut manager, DEMO_ID, initialize_request(41), &batches, &mut panels).expect("initialize guest");
        assert_eq!(manager.storage_snapshot(DEMO_ID).expect("granted storage"), Some(Vec::new()));
        batches.write().expect("action queue").clear();
        let toggle_request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 42,
            event: ExtensionEvent::PanelAction {
                panel_id: "sandbox-demo".into(),
                control_id: "enabled".into(),
                value: PanelEventValue::Toggled { value: true },
            },
            storage: None,
        };
        dispatch_extension(&mut manager, DEMO_ID, toggle_request, &batches, &mut panels).expect("persist toggle");
        let actions = batches
            .write()
            .expect("action queue")
            .pop_front()
            .expect("UI response batch")
            .actions;
        assert!(actions.iter().any(|action| matches!(action, ExtensionAction::RegisterPanel { .. })));
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, ExtensionAction::ShowNotification { .. }))
        );
        assert!(!actions.iter().any(|action| matches!(action, ExtensionAction::SetStorage { .. })));
        assert_eq!(
            manager.storage_snapshot(DEMO_ID).expect("persisted storage"),
            Some(vec![aurora_extension_api::ExtensionStorageEntry {
                key: "ui.enabled".into(),
                value: "true".into(),
            }])
        );

        drop(manager);
        let reopened = PersistentExtensionManager::open(&temp.0, Version::new(1, 0, 0)).expect("reopen extension manager");
        assert_eq!(
            reopened.storage_snapshot(DEMO_ID).expect("restored extension storage"),
            Some(vec![aurora_extension_api::ExtensionStorageEntry {
                key: "ui.enabled".into(),
                value: "true".into(),
            }])
        );
    }

    #[test]
    fn ungranted_notification_action_is_quarantined_and_reported_in_manager() {
        let temp = TempRoot::new("extension-notification-denied");
        let permissions = [Permission::UiPanelRegister];
        let (mut manager, _) = install_fixture(&temp.0, &permissions);
        let batches = RwLock::new(VecDeque::new());
        let mut panels = Vec::new();
        dispatch_extension(&mut manager, DEMO_ID, initialize_request(41), &batches, &mut panels).expect("initialize guest");
        batches.write().expect("action queue").clear();

        let error = dispatch_extension(&mut manager, DEMO_ID, panel_request(42), &batches, &mut panels)
            .expect_err("ungranted notification capability must be rejected");
        assert!(error.contains(DEMO_ID));
        assert_eq!(manager.list()[0].state, ExtensionState::Quarantined);
        assert!(panels.is_empty());

        let shared = RwLock::new(Snapshot::default());
        publish_failure(&shared, Some(&manager), &panels, error);
        let snapshot = shared.read().expect("manager snapshot").clone();
        assert!(
            snapshot.error.is_some(),
            "execution failures must be visible in Preferences › Extensions"
        );
        assert_eq!(snapshot.installed[0].state, ExtensionState::Quarantined);
    }

    #[test]
    fn account_scopes_isolate_grants_and_discard_stale_commands() {
        let temp = TempRoot::new("extension-account-scope");
        let account_a = account_scope_key(
            "https://grid-a.example/login",
            Some(uuid::Uuid::from_u128(0x123456789abcdef0123456789abcdef0)),
        );
        let account_b = account_scope_key(
            "https://grid-a.example/login",
            Some(uuid::Uuid::from_u128(0xabcdef0123456789abcdef0123456789)),
        );
        let package_path = temp.0.join("fixture.ext");
        std::fs::create_dir_all(&temp.0).expect("create temporary store root");
        std::fs::write(&package_path, fixture_package_with_request_ids(&["extension.full_access"], 1, 2))
            .expect("write local test package");
        let service = Service::new(temp.0.clone());

        assert!(service.submit(Command::SetAccountScope(account_a.clone())));
        let initial = wait_for_snapshot(&service, &account_a, |snapshot| snapshot.ready || snapshot.error.is_some());
        assert!(initial.ready, "account A manager should open: {:?}", initial.error);
        assert!(service.submit(Command::ChooseInstall(package_path.clone())));
        let review = wait_for_snapshot(&service, &account_a, |snapshot| {
            snapshot.pending.is_some() || snapshot.error.is_some()
        });
        assert!(review.pending.is_some(), "package review should be available: {:?}", review.error);
        assert!(service.submit(Command::CommitInstall {
            permissions: vec![Permission::FullAccess.manifest_key().to_owned()],
            enable: true,
            acknowledge_unverified: true,
            acknowledge_publisher_change: false,
        }));
        let installed = wait_for_snapshot(&service, &account_a, |snapshot| {
            (snapshot.installed.len() == 1 && snapshot.panels.len() == 1) || snapshot.error.is_some()
        });
        assert!(installed.error.is_none(), "install/initialize failed: {:?}", installed.error);
        assert_eq!(installed.installed[0].state, ExtensionState::Enabled);
        assert_eq!(installed.installed[0].granted_permissions, vec![Permission::FullAccess]);

        assert!(service.submit(Command::SetAccountScope(account_b.clone())));
        assert!(
            service.take_actions_for_scope(&account_b).is_empty(),
            "old-account guest effects must be discarded"
        );
        let other_account = wait_for_snapshot(&service, &account_b, |snapshot| snapshot.ready || snapshot.error.is_some());
        assert!(other_account.ready, "account B manager should open: {:?}", other_account.error);
        assert!(
            other_account.installed.is_empty(),
            "packages and grants must not cross account scopes"
        );
        assert!(other_account.panels.is_empty());

        assert!(service.submit(Command::ChooseInstall(package_path)));
        let review_b = wait_for_snapshot(&service, &account_b, |snapshot| {
            snapshot.pending.is_some() || snapshot.error.is_some()
        });
        assert!(
            review_b.pending.is_some(),
            "account B package review should be available: {:?}",
            review_b.error
        );
        assert!(service.submit(Command::CommitInstall {
            permissions: vec![Permission::FullAccess.manifest_key().to_owned()],
            enable: true,
            acknowledge_unverified: true,
            acknowledge_publisher_change: false,
        }));
        let installed_b = wait_for_snapshot(&service, &account_b, |snapshot| {
            (snapshot.installed.len() == 1 && snapshot.panels.len() == 1) || snapshot.error.is_some()
        });
        assert!(installed_b.error.is_none(), "account B install failed: {:?}", installed_b.error);
        assert_eq!(installed_b.installed[0].state, ExtensionState::Enabled);
        assert_eq!(
            service.take_actions_for_scope(&account_b).len(),
            1,
            "drain account B initialization effects"
        );

        assert!(service.submit_for_scope(
            &account_a,
            Command::PanelAction {
                id: DEMO_ID.into(),
                panel_id: "sandbox-demo".into(),
                control_id: "notify".into(),
                value: PanelEventValue::Clicked,
            }
        ));
        assert!(service.submit_for_scope(
            &account_a,
            Command::NearbyChat(ExtensionEvent::NearbyChat {
                message_id: 99,
                speaker: "Account A".into(),
                text: "stale event".into(),
            })
        ));
        assert!(service.submit_for_scope(&account_b, Command::SafeMode(true)));
        let after_stale_commands = wait_for_snapshot(&service, &account_b, |snapshot| snapshot.safe_mode || snapshot.error.is_some());
        assert!(
            after_stale_commands.error.is_none(),
            "stale account commands failed: {:?}",
            after_stale_commands.error
        );
        assert!(after_stale_commands.safe_mode);
        assert_eq!(after_stale_commands.installed[0].state, ExtensionState::Enabled);
        assert!(
            service.take_actions_for_scope(&account_b).is_empty(),
            "stale account effects must not be applied"
        );

        assert!(service.submit(Command::SetAccountScope(account_a.clone())));
        let restored = wait_for_snapshot(&service, &account_a, |snapshot| {
            (snapshot.ready && snapshot.installed.len() == 1 && snapshot.panels.len() == 1) || snapshot.error.is_some()
        });
        assert!(restored.error.is_none(), "account A restore failed: {:?}", restored.error);
        assert_eq!(restored.installed[0].state, ExtensionState::Enabled);
        assert_eq!(restored.installed[0].granted_permissions, vec![Permission::FullAccess]);

        // Leave the worker in safe mode so its final thread shutdown cannot
        // dispatch a fixture Shutdown event after the assertions.
        assert!(service.submit(Command::SetAccountScope(account_b.clone())));
        let restored_b = wait_for_snapshot(&service, &account_b, |snapshot| snapshot.ready || snapshot.error.is_some());
        assert!(restored_b.ready, "account B manager should remain loadable: {:?}", restored_b.error);
    }
}
