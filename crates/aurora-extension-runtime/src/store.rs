use crate::{
    ExtensionManager, ExtensionState, ExtensionSummary, ManagerError, PublisherKeyFile, StagedExtension, TrustedPublisher,
    parse_public_key_file, public_key_file,
};
use aurora_extension_api::{
    API_VERSION, ExtensionAction, ExtensionEvent, ExtensionRequest, ExtensionStorageEntry, MAX_STORAGE_ENTRIES, Permission,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const REGISTRY_VERSION: u32 = 1;
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_INSTALLED_EXTENSIONS: usize = 32;
const TRUST_REGISTRY_VERSION: u32 = 1;
const MAX_TRUST_REGISTRY_BYTES: u64 = 256 * 1024;
const MAX_TRUSTED_PUBLISHERS: usize = 256;
const MAX_PUBLISHER_LABEL_BYTES: usize = 128;
const TRUST_REGISTRY_FILE: &str = "trusted_publishers.json";
const EXTENSION_STORAGE_VERSION: u32 = 1;
const MAX_EXTENSION_STORAGE_BYTES: u64 = 8 * 1024;
const EXTENSION_STORAGE_DIR: &str = "extension_data";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Failure while loading or durably updating the per-user extension registry.
#[derive(Debug)]
pub enum PersistentManagerError {
    /// Package validation or lifecycle failure.
    Manager(ManagerError),
    /// Registry or package storage failed or contained invalid data.
    Storage(String),
}

impl fmt::Display for PersistentManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manager(error) => write!(f, "{error}"),
            Self::Storage(error) => write!(f, "extension storage error: {error}"),
        }
    }
}
impl Error for PersistentManagerError {}
impl PersistentManagerError {
    /// Return a security-block reason from the package validator, if this error represents one.
    pub fn blocked_reason(&self) -> Option<&str> {
        match self {
            Self::Manager(ManagerError::Package(error)) => error.blocked_reason(),
            Self::Manager(_) | Self::Storage(_) => None,
        }
    }
}
impl From<ManagerError> for PersistentManagerError {
    fn from(error: ManagerError) -> Self {
        Self::Manager(error)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    safe_mode: bool,
    extensions: Vec<RegistryEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegistryEntry {
    id: String,
    archive_sha256: String,
    content_sha256: String,
    permissions: Vec<String>,
    state: StoredState,
    /// Phase 1 installed packages were explicitly accepted after their unverified publisher claim was shown.
    #[serde(default = "legacy_unverified_acknowledged")]
    unverified_acknowledged: bool,
}

fn legacy_unverified_acknowledged() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustRegistry {
    schema_version: u32,
    publishers: Vec<TrustedPublisherEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExtensionStorageFile {
    schema_version: u32,
    entries: Vec<ExtensionStorageEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustedPublisherEntry {
    key_id: String,
    public_key: String,
    label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredState {
    Disabled,
    Enabled,
    Quarantined,
}

/// Persistent package/grant manager. Packages are immutable content-addressed files;
/// registry updates are written atomically and contain no guest code.
pub struct PersistentExtensionManager {
    root: PathBuf,
    supported_api: Version,
    manager: ExtensionManager,
    archive_hashes: BTreeMap<String, String>,
    registry: Registry,
    trusted_publishers: Vec<TrustedPublisher>,
    #[cfg(test)]
    registry_save_fail_after: std::cell::Cell<Option<usize>>,
}

impl PersistentExtensionManager {
    /// Open or create an extension store under a per-user data directory.
    pub fn open(root: impl Into<PathBuf>, supported_api: Version) -> Result<Self, PersistentManagerError> {
        let root = root.into();
        fs::create_dir_all(root.join("packages")).map_err(storage_error)?;
        let trusted_publishers = load_trusted_publishers(&root)?;
        let registry_path = root.join("registry.json");
        let mut registry = if registry_path.exists() {
            let metadata = fs::metadata(&registry_path).map_err(storage_error)?;
            if metadata.len() > MAX_REGISTRY_BYTES {
                return Err(storage("registry exceeds size limit"));
            }
            let bytes = fs::read(&registry_path).map_err(storage_error)?;
            serde_json::from_slice::<Registry>(&bytes).map_err(|_| storage("registry is malformed"))?
        } else {
            Registry {
                schema_version: REGISTRY_VERSION,
                safe_mode: false,
                extensions: Vec::new(),
            }
        };
        if registry.schema_version != REGISTRY_VERSION || registry.extensions.len() > MAX_INSTALLED_EXTENSIONS {
            return Err(storage("registry version or extension count is unsupported"));
        }

        let mut manager = ExtensionManager::new(supported_api.clone());
        manager.set_trusted_publishers(&trusted_publishers);
        let mut archive_hashes = BTreeMap::new();
        let mut ids = HashSet::new();
        let mut disabled_for_missing_acknowledgement = false;
        for index in 0..registry.extensions.len() {
            let entry = registry.extensions[index].clone();
            if !valid_extension_id(&entry.id)
                || !valid_digest(&entry.archive_sha256)
                || !valid_digest(&entry.content_sha256)
                || !ids.insert(entry.id.clone())
            {
                return Err(storage("registry contains an invalid or duplicate extension record"));
            }
            let path = package_path(&root, &entry.id, &entry.archive_sha256);
            let metadata = fs::metadata(&path).map_err(storage_error)?;
            if metadata.len() > crate::MAX_PACKAGE_BYTES as u64 {
                return Err(storage("stored package exceeds size limit"));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            fs::File::open(&path)
                .and_then(|mut file| file.read_to_end(&mut bytes))
                .map_err(storage_error)?;
            if digest_hex(&bytes) != entry.archive_sha256 {
                return Err(storage("stored package hash does not match registry"));
            }
            let staged = manager.stage_install(&bytes)?;
            if staged.review().manifest.id != entry.id || digest_hex(&staged.review().content_sha256) != entry.content_sha256 {
                return Err(storage("stored package identity does not match registry"));
            }
            let grants = parse_permissions(&entry.permissions)?;
            manager.install_with_ack(staged, &grants, false, entry.unverified_acknowledged, true)?;
            match entry.state {
                StoredState::Disabled => {}
                StoredState::Enabled => match manager.enable_with_ack(&entry.id, &grants, entry.unverified_acknowledged) {
                    Ok(()) => {}
                    Err(ManagerError::UnverifiedAcknowledgementRequired) => {
                        registry.extensions[index].state = StoredState::Disabled;
                        disabled_for_missing_acknowledgement = true;
                    }
                    Err(error) => return Err(error.into()),
                },
                StoredState::Quarantined => manager.restore_quarantine(&entry.id)?,
            }
            archive_hashes.insert(entry.id.clone(), entry.archive_sha256.clone());
        }
        manager.set_safe_mode(registry.safe_mode);
        let store = Self {
            root,
            supported_api,
            manager,
            archive_hashes,
            registry,
            trusted_publishers,
            #[cfg(test)]
            registry_save_fail_after: std::cell::Cell::new(None),
        };
        if disabled_for_missing_acknowledgement {
            store.save_registry()?;
        }
        Ok(store)
    }

    /// Return safe metadata for locally trusted publisher keys.
    pub fn trusted_publishers(&self) -> &[TrustedPublisher] {
        &self.trusted_publishers
    }

    /// Add or update a publisher key only after the caller has shown its fingerprint for user consent.
    pub fn trust_publisher(&mut self, key_file_bytes: &[u8], label: &str) -> Result<TrustedPublisher, PersistentManagerError> {
        if label.trim().is_empty() || label.len() > MAX_PUBLISHER_LABEL_BYTES || label.chars().any(char::is_control) {
            return Err(storage("publisher label is invalid"));
        }
        let verified = parse_public_key_file(key_file_bytes).map_err(|error| storage(&error.to_string()))?;
        let publisher = TrustedPublisher {
            key_id: verified.key_id,
            public_key: verified.public_key,
            label: label.trim().to_owned(),
        };
        let previous = self.trusted_publishers.clone();
        if let Some(existing) = self.trusted_publishers.iter_mut().find(|entry| entry.key_id == publisher.key_id) {
            if existing.public_key != publisher.public_key {
                return Err(storage("publisher fingerprint collision"));
            }
            existing.label = publisher.label.clone();
        } else {
            if self.trusted_publishers.len() >= MAX_TRUSTED_PUBLISHERS {
                return Err(storage("trusted publisher limit reached"));
            }
            self.trusted_publishers.push(publisher.clone());
            self.trusted_publishers.sort_by(|a, b| a.key_id.cmp(&b.key_id));
        }
        if let Err(error) = self.save_trusted_publishers() {
            self.trusted_publishers = previous;
            return Err(error);
        }
        self.manager.set_trusted_publishers(&self.trusted_publishers);
        Ok(publisher)
    }

    /// Explicitly revoke one key, disabling its packages and clearing old unverified acknowledgements.
    pub fn untrust_publisher(&mut self, key_id: &str) -> Result<(), PersistentManagerError> {
        let previous_trusted = self.trusted_publishers.clone();
        let previous_registry = self.registry.clone();
        let old_len = self.trusted_publishers.len();
        self.trusted_publishers.retain(|publisher| publisher.key_id != key_id);
        if self.trusted_publishers.len() == old_len {
            return Err(storage("publisher key is not trusted"));
        }
        if let Err(error) = self.save_trusted_publishers() {
            self.trusted_publishers = previous_trusted;
            return Err(error);
        }
        self.manager.set_trusted_publishers(&self.trusted_publishers);
        self.manager.revoke_publisher(key_id);
        for entry in &mut self.registry.extensions {
            let summary = self.manager.summary(&entry.id)?;
            entry.state = state_from_summary(&summary);
            entry.unverified_acknowledged = summary.unverified_acknowledged;
        }
        if let Err(error) = self.save_registry() {
            self.trusted_publishers = previous_trusted;
            self.registry = previous_registry;
            let _ = self.save_trusted_publishers();
            let _ = self.reload_manager();
            return Err(error);
        }
        Ok(())
    }

    /// Validate and stage a package without installing it or granting permissions.
    pub fn stage_install(&self, bytes: &[u8]) -> Result<StagedExtension, PersistentManagerError> {
        if self.registry.extensions.len() >= MAX_INSTALLED_EXTENSIONS {
            return Err(storage("installed extension limit reached"));
        }
        Ok(self.manager.stage_install(bytes)?)
    }

    /// Commit a reviewed package only after the displayed trust and permissions are acknowledged.
    pub fn install(
        &mut self,
        staged: StagedExtension,
        package_bytes: &[u8],
        approved_permissions: &[Permission],
        enable: bool,
    ) -> Result<ExtensionSummary, PersistentManagerError> {
        self.install_with_ack(staged, package_bytes, approved_permissions, enable, false, false)
    }

    /// Commit a reviewed package with explicit unverified-publisher and key-change acknowledgement.
    pub fn install_with_ack(
        &mut self,
        staged: StagedExtension,
        package_bytes: &[u8],
        approved_permissions: &[Permission],
        enable: bool,
        acknowledge_unverified: bool,
        acknowledge_publisher_change: bool,
    ) -> Result<ExtensionSummary, PersistentManagerError> {
        let verified = self.manager.stage_install(package_bytes)?;
        if verified.review() != staged.review() {
            return Err(storage("staged package does not match install bytes"));
        }
        if matches!(
            &staged.review().publisher_trust,
            crate::PublisherTrustStatus::UnverifiedPlugin { .. }
        ) && !acknowledge_unverified
        {
            return Err(ManagerError::UnverifiedAcknowledgementRequired.into());
        }
        if staged.review().publisher_identity_changed && !acknowledge_publisher_change {
            return Err(ManagerError::PublisherIdentityChanged.into());
        }
        let review = staged.review().clone();
        let id = review.manifest.id.clone();
        let archive_hash = digest_hex(package_bytes);
        let path = package_path(&self.root, &id, &archive_hash);
        write_immutable(&path, package_bytes)?;
        let summary = match self.manager.install_with_ack(
            staged,
            approved_permissions,
            enable,
            acknowledge_unverified,
            acknowledge_publisher_change,
        ) {
            Ok(summary) => summary,
            Err(error) => {
                remove_if_unreferenced(&path);
                return Err(error.into());
            }
        };
        self.archive_hashes.insert(id.clone(), archive_hash.clone());
        self.registry.extensions.push(entry_from_summary(&summary, archive_hash));
        if let Err(error) = self.save_registry() {
            let _ = self.manager.uninstall(&id);
            self.archive_hashes.remove(&id);
            self.registry.extensions.retain(|entry| entry.id != id);
            remove_if_unreferenced(&path);
            return Err(error);
        }
        Ok(summary)
    }

    /// Validate an update for an installed ID without changing packages or grants.
    pub fn stage_update(&self, id: &str, bytes: &[u8]) -> Result<StagedExtension, PersistentManagerError> {
        Ok(self.manager.stage_update(id, bytes)?)
    }

    /// Commit a newer package only after explicitly reviewing permission and publisher changes.
    pub fn update(
        &mut self,
        id: &str,
        staged: StagedExtension,
        package_bytes: &[u8],
        approved_permissions: &[Permission],
        enable: bool,
    ) -> Result<ExtensionSummary, PersistentManagerError> {
        self.update_with_ack(id, staged, package_bytes, approved_permissions, enable, false, false)
    }

    /// Commit an update after explicit acknowledgement of unverified or changed publisher identity.
    pub fn update_with_ack(
        &mut self,
        id: &str,
        staged: StagedExtension,
        package_bytes: &[u8],
        approved_permissions: &[Permission],
        enable: bool,
        acknowledge_unverified: bool,
        acknowledge_publisher_change: bool,
    ) -> Result<ExtensionSummary, PersistentManagerError> {
        let verified = self.manager.stage_update(id, package_bytes)?;
        if verified.review() != staged.review() {
            return Err(storage("staged update does not match install bytes"));
        }
        if matches!(
            &staged.review().publisher_trust,
            crate::PublisherTrustStatus::UnverifiedPlugin { .. }
        ) && !acknowledge_unverified
        {
            return Err(ManagerError::UnverifiedAcknowledgementRequired.into());
        }
        if staged.review().publisher_identity_changed && !acknowledge_publisher_change {
            return Err(ManagerError::PublisherIdentityChanged.into());
        }
        let review = staged.review().clone();
        let requested: HashSet<_> = review.requested_permissions.iter().copied().collect();
        let approved: HashSet<_> = approved_permissions.iter().copied().collect();
        if approved.len() != approved_permissions.len() || !approved.is_subset(&requested) {
            return Err(ManagerError::InvalidPermissionSet.into());
        }
        if enable && approved != requested {
            return Err(ManagerError::MissingPermissions.into());
        }
        if enable && self.registry.safe_mode {
            return Err(ManagerError::SafeMode.into());
        }

        let entry_index = self
            .registry
            .extensions
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(ManagerError::NotInstalled)?;
        let old_entry = self.registry.extensions[entry_index].clone();
        let old_hash = self
            .archive_hashes
            .get(id)
            .cloned()
            .ok_or_else(|| storage("package hash is missing"))?;
        let archive_hash = digest_hex(package_bytes);
        let new_package_path = package_path(&self.root, id, &archive_hash);
        write_immutable(&new_package_path, package_bytes)?;

        let mut permissions: Vec<_> = approved_permissions
            .iter()
            .map(|permission| permission.manifest_key().to_owned())
            .collect();
        permissions.sort();
        self.registry.extensions[entry_index] = RegistryEntry {
            id: id.to_owned(),
            archive_sha256: archive_hash.clone(),
            content_sha256: digest_hex(&review.content_sha256),
            permissions,
            state: if enable { StoredState::Enabled } else { StoredState::Disabled },
            unverified_acknowledged: acknowledge_unverified,
        };
        self.archive_hashes.insert(id.to_owned(), archive_hash);
        if let Err(error) = self.save_registry() {
            self.registry.extensions[entry_index] = old_entry;
            self.archive_hashes.insert(id.to_owned(), old_hash);
            remove_if_unreferenced(&new_package_path);
            return Err(error);
        }

        match self.manager.apply_update_with_ack(
            staged,
            approved_permissions,
            enable,
            acknowledge_unverified,
            acknowledge_publisher_change,
        ) {
            Ok(summary) => {
                let _ = fs::remove_file(package_path(&self.root, id, &old_hash));
                Ok(summary)
            }
            Err(error) => {
                self.registry.extensions[entry_index] = old_entry;
                self.archive_hashes.insert(id.to_owned(), old_hash.clone());
                if let Err(rollback_error) = self.save_registry() {
                    return match self.reload_manager() {
                        Ok(()) => {
                            remove_if_unreferenced(&package_path(&self.root, id, &old_hash));
                            Err(rollback_error)
                        }
                        Err(reload_error) => Err(storage(&format!(
                            "update rollback failed ({rollback_error}); reopening persisted state also failed ({reload_error})"
                        ))),
                    };
                }
                self.reload_manager()?;
                remove_if_unreferenced(&new_package_path);
                Err(error.into())
            }
        }
    }

    /// Enable only after explicit permission grants and, when needed, publisher acknowledgement.
    pub fn enable(&mut self, id: &str, approved: &[Permission]) -> Result<(), PersistentManagerError> {
        self.enable_with_ack(id, approved, false)
    }

    /// Enable an unverified plugin only after an explicit user acknowledgement.
    pub fn enable_with_ack(
        &mut self,
        id: &str,
        approved: &[Permission],
        acknowledge_unverified: bool,
    ) -> Result<(), PersistentManagerError> {
        let summary = self.manager.summary(id)?;
        let approved_set: HashSet<_> = approved.iter().copied().collect();
        let requested: HashSet<_> = summary.requested_permissions.iter().copied().collect();
        if summary.state == ExtensionState::Quarantined {
            return Err(ManagerError::Quarantined.into());
        }
        if approved_set.len() != approved.len() || approved_set != requested {
            return Err(ManagerError::MissingPermissions.into());
        }
        if self.registry.safe_mode {
            return Err(ManagerError::SafeMode.into());
        }
        let old_entry = self.entry_mut(id)?.clone();
        let entry = self.entry_mut(id)?;
        entry.permissions = approved.iter().map(|permission| permission.manifest_key().to_owned()).collect();
        entry.permissions.sort();
        entry.state = StoredState::Enabled;
        entry.unverified_acknowledged |= acknowledge_unverified;
        if let Err(error) = self.save_registry() {
            *self.entry_mut(id)? = old_entry;
            return Err(error);
        }
        if let Err(error) = self.manager.enable_with_ack(id, approved, acknowledge_unverified) {
            *self.entry_mut(id)? = old_entry;
            self.save_registry()?;
            return Err(error.into());
        }
        Ok(())
    }

    /// Disable an extension while retaining its grants.
    pub fn disable(&mut self, id: &str) -> Result<(), PersistentManagerError> {
        let summary = self.manager.summary(id)?;
        let old_entry = self.entry_mut(id)?.clone();
        self.entry_mut(id)?.state = if summary.state == ExtensionState::Quarantined {
            StoredState::Quarantined
        } else {
            StoredState::Disabled
        };
        if let Err(error) = self.save_registry() {
            *self.entry_mut(id)? = old_entry;
            return Err(error);
        }
        self.manager.disable(id)?;
        Ok(())
    }

    /// Revoke a granted capability; any revocation disables the extension.
    pub fn revoke_permission(&mut self, id: &str, permission: Permission) -> Result<(), PersistentManagerError> {
        let summary = self.manager.summary(id)?;
        if !summary.granted_permissions.contains(&permission) {
            return Err(ManagerError::PermissionNotGranted.into());
        }
        let old_entry = self.entry_mut(id)?.clone();
        let entry = self.entry_mut(id)?;
        entry
            .permissions
            .retain(|key| Permission::from_manifest_key(key) != Some(permission));
        entry.state = StoredState::Disabled;
        if let Err(error) = self.save_registry() {
            *self.entry_mut(id)? = old_entry;
            return Err(error);
        }
        self.manager.revoke_permission(id, permission)?;
        Ok(())
    }

    /// Uninstall by removing registry access first; orphan cleanup is best-effort.
    pub fn uninstall(&mut self, id: &str) -> Result<(), PersistentManagerError> {
        self.manager.summary(id)?;
        let hash = self
            .archive_hashes
            .get(id)
            .cloned()
            .ok_or_else(|| storage("package hash is missing"))?;
        let old_registry = self.registry.clone();
        self.registry.extensions.retain(|entry| entry.id != id);
        self.archive_hashes.remove(id);
        if let Err(error) = self.save_registry() {
            self.registry = old_registry;
            self.archive_hashes.insert(id.to_owned(), hash);
            return Err(error);
        }
        self.manager.uninstall(id)?;
        let _ = fs::remove_file(package_path(&self.root, id, &hash));
        match fs::remove_file(extension_storage_path(&self.root, id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
        Ok(())
    }

    /// Persist safe mode before applying it to the in-memory manager.
    pub fn set_safe_mode(&mut self, enabled: bool) -> Result<(), PersistentManagerError> {
        let previous = self.registry.safe_mode;
        self.registry.safe_mode = enabled;
        if let Err(error) = self.save_registry() {
            self.registry.safe_mode = previous;
            return Err(error);
        }
        self.manager.set_safe_mode(enabled);
        Ok(())
    }

    /// Return this extension's persisted values only when its explicit storage grant is active.
    pub fn storage_snapshot(&self, id: &str) -> Result<Option<Vec<ExtensionStorageEntry>>, PersistentManagerError> {
        let summary = self.manager.summary(id)?;
        if !summary.granted_permissions.contains(&Permission::ExtensionStorage) {
            return Ok(None);
        }
        self.read_extension_storage(id).map(Some)
    }

    /// Dispatch one event and persist quarantine after a guest trap or capability violation.
    pub fn dispatch(
        &mut self,
        id: &str,
        request: &aurora_extension_api::ExtensionRequest,
    ) -> Result<Vec<aurora_extension_api::ExtensionAction>, PersistentManagerError> {
        let result = self.manager.dispatch(id, request);
        if matches!(
            result.as_ref(),
            Err(ManagerError::RuntimeFailure | ManagerError::CapabilityViolation)
        ) {
            let summary = self.manager.summary(id)?;
            self.entry_mut(id)?.state = StoredState::Quarantined;
            self.save_registry()?;
            debug_assert_eq!(state_from_summary(&summary), StoredState::Quarantined);
        }
        let mut actions = result?;
        if actions.iter().any(|action| matches!(action, ExtensionAction::SetStorage { .. })) {
            let current = self.read_extension_storage(id)?;
            let mut values: BTreeMap<_, _> = current.into_iter().map(|entry| (entry.key, entry.value)).collect();
            for action in &actions {
                if let ExtensionAction::SetStorage { key, value } = action {
                    if let Some(value) = value {
                        values.insert(key.clone(), value.clone());
                    } else {
                        values.remove(key);
                    }
                }
            }
            self.write_extension_storage(
                id,
                values
                    .into_iter()
                    .map(|(key, value)| ExtensionStorageEntry { key, value })
                    .collect(),
            )?;
            actions.retain(|action| !matches!(action, ExtensionAction::SetStorage { .. }));
        }
        Ok(actions)
    }

    /// Clear quarantine after user review; the extension remains disabled.
    pub fn clear_quarantine(&mut self, id: &str) -> Result<(), PersistentManagerError> {
        let summary = self.manager.summary(id)?;
        if summary.state == ExtensionState::Quarantined {
            let old_entry = self.entry_mut(id)?.clone();
            self.entry_mut(id)?.state = StoredState::Disabled;
            if let Err(error) = self.save_registry() {
                *self.entry_mut(id)? = old_entry;
                return Err(error);
            }
        }
        self.manager.clear_quarantine(id)?;
        Ok(())
    }

    /// Whether extension execution is globally disabled.
    pub fn safe_mode(&self) -> bool {
        self.registry.safe_mode
    }

    #[cfg(test)]
    pub(crate) fn inject_registry_save_failure_after(&self, successful_saves: usize) {
        self.registry_save_fail_after.set(Some(successful_saves));
    }

    #[cfg(test)]
    pub(crate) fn set_manager_safe_mode_for_test(&mut self, enabled: bool) {
        self.manager.set_safe_mode(enabled);
    }

    /// Safe metadata for installed extensions.
    pub fn list(&self) -> Vec<ExtensionSummary> {
        self.manager.list()
    }

    fn reload_manager(&mut self) -> Result<(), PersistentManagerError> {
        let reopened = Self::open(self.root.clone(), self.supported_api.clone())?;
        self.manager = reopened.manager;
        self.archive_hashes = reopened.archive_hashes;
        self.registry = reopened.registry;
        self.trusted_publishers = reopened.trusted_publishers;
        Ok(())
    }

    fn entry_mut(&mut self, id: &str) -> Result<&mut RegistryEntry, PersistentManagerError> {
        self.registry
            .extensions
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| storage("extension is not in the registry"))
    }

    fn read_extension_storage(&self, id: &str) -> Result<Vec<ExtensionStorageEntry>, PersistentManagerError> {
        let path = extension_storage_path(&self.root, id);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(storage_error(error)),
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_EXTENSION_STORAGE_BYTES {
            return Err(storage("extension storage file is invalid or exceeds its size limit"));
        }
        let file: ExtensionStorageFile =
            serde_json::from_slice(&fs::read(path).map_err(storage_error)?).map_err(|_| storage("extension storage file is malformed"))?;
        if file.schema_version != EXTENSION_STORAGE_VERSION || file.entries.len() > MAX_STORAGE_ENTRIES {
            return Err(storage("extension storage schema or entry count is unsupported"));
        }
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 0,
            event: ExtensionEvent::Shutdown,
            storage: Some(file.entries.clone()),
        };
        request.validate().map_err(|_| storage("extension storage entries are invalid"))?;
        Ok(file.entries)
    }

    fn write_extension_storage(&self, id: &str, entries: Vec<ExtensionStorageEntry>) -> Result<(), PersistentManagerError> {
        let path = extension_storage_path(&self.root, id);
        if entries.is_empty() {
            return match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(storage_error(error)),
            };
        }
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 0,
            event: ExtensionEvent::Shutdown,
            storage: Some(entries.clone()),
        };
        request
            .validate()
            .map_err(|_| storage("extension storage entries exceed their limits"))?;
        let file = ExtensionStorageFile {
            schema_version: EXTENSION_STORAGE_VERSION,
            entries,
        };
        let mut bytes = serde_json::to_vec_pretty(&file).map_err(|_| storage("could not serialize extension storage"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_EXTENSION_STORAGE_BYTES {
            return Err(storage("extension storage quota exceeded"));
        }
        fs::create_dir_all(self.root.join(EXTENSION_STORAGE_DIR)).map_err(storage_error)?;
        atomic_write(&path, &bytes)
    }

    fn save_registry(&self) -> Result<(), PersistentManagerError> {
        #[cfg(test)]
        if let Some(remaining) = self.registry_save_fail_after.get() {
            if remaining == 0 {
                self.registry_save_fail_after.set(None);
                return Err(storage("injected registry save failure"));
            }
            self.registry_save_fail_after.set(Some(remaining - 1));
        }
        let mut bytes = serde_json::to_vec_pretty(&self.registry).map_err(|_| storage("could not serialize registry"))?;
        bytes.push(b'\n');
        atomic_write(&self.root.join("registry.json"), &bytes)
    }

    fn save_trusted_publishers(&self) -> Result<(), PersistentManagerError> {
        let registry = TrustRegistry {
            schema_version: TRUST_REGISTRY_VERSION,
            publishers: self
                .trusted_publishers
                .iter()
                .map(|publisher| {
                    let key_file = public_key_file(&publisher.public_key);
                    TrustedPublisherEntry {
                        key_id: key_file.key_id,
                        public_key: key_file.public_key,
                        label: publisher.label.clone(),
                    }
                })
                .collect(),
        };
        let mut bytes = serde_json::to_vec_pretty(&registry).map_err(|_| storage("could not serialize trusted publishers"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_TRUST_REGISTRY_BYTES {
            return Err(storage("trusted publisher registry exceeds size limit"));
        }
        atomic_write(&self.root.join(TRUST_REGISTRY_FILE), &bytes)
    }
}

fn load_trusted_publishers(root: &Path) -> Result<Vec<TrustedPublisher>, PersistentManagerError> {
    let path = root.join(TRUST_REGISTRY_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let metadata = fs::metadata(&path).map_err(storage_error)?;
    if metadata.len() > MAX_TRUST_REGISTRY_BYTES {
        return Err(storage("trusted publisher registry exceeds size limit"));
    }
    let bytes = fs::read(path).map_err(storage_error)?;
    let registry: TrustRegistry = serde_json::from_slice(&bytes).map_err(|_| storage("trusted publisher registry is malformed"))?;
    if registry.schema_version != TRUST_REGISTRY_VERSION || registry.publishers.len() > MAX_TRUSTED_PUBLISHERS {
        return Err(storage("trusted publisher registry version or count is unsupported"));
    }
    let mut publishers = BTreeMap::new();
    for entry in registry.publishers {
        if entry.label.trim().is_empty() || entry.label.len() > MAX_PUBLISHER_LABEL_BYTES || entry.label.chars().any(char::is_control) {
            return Err(storage("trusted publisher registry contains an invalid label"));
        }
        let key_file = PublisherKeyFile {
            schema_version: TRUST_REGISTRY_VERSION,
            algorithm: "ed25519".into(),
            key_id: entry.key_id,
            public_key: entry.public_key,
        };
        let key_bytes = serde_json::to_vec(&key_file).map_err(|_| storage("could not validate trusted publisher key"))?;
        let verified = parse_public_key_file(&key_bytes).map_err(|_| storage("trusted publisher fingerprint does not match its key"))?;
        let publisher = TrustedPublisher {
            key_id: verified.key_id.clone(),
            public_key: verified.public_key,
            label: entry.label,
        };
        if publishers.insert(publisher.key_id.clone(), publisher).is_some() {
            return Err(storage("trusted publisher registry contains duplicate keys"));
        }
    }
    Ok(publishers.into_values().collect())
}

fn entry_from_summary(summary: &ExtensionSummary, archive_sha256: String) -> RegistryEntry {
    let mut permissions: Vec<_> = summary
        .granted_permissions
        .iter()
        .map(|permission| permission.manifest_key().to_owned())
        .collect();
    permissions.sort();
    RegistryEntry {
        id: summary.manifest.id.clone(),
        archive_sha256,
        content_sha256: digest_hex(&summary.content_sha256),
        permissions,
        state: state_from_summary(summary),
        unverified_acknowledged: summary.unverified_acknowledged,
    }
}

fn state_from_summary(summary: &ExtensionSummary) -> StoredState {
    match summary.state {
        ExtensionState::Disabled => StoredState::Disabled,
        ExtensionState::Enabled => StoredState::Enabled,
        ExtensionState::Quarantined => StoredState::Quarantined,
    }
}

fn parse_permissions(keys: &[String]) -> Result<Vec<Permission>, PersistentManagerError> {
    let mut seen = HashSet::new();
    keys.iter()
        .map(|key| {
            let permission = Permission::from_manifest_key(key).ok_or_else(|| storage("registry contains an unknown permission"))?;
            if !seen.insert(permission) {
                return Err(storage("registry contains a duplicate permission"));
            }
            Ok(permission)
        })
        .collect()
}

fn valid_extension_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.is_ascii()
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && part.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && part.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn package_path(root: &Path, id: &str, digest: &str) -> PathBuf {
    root.join("packages").join(format!("{id}-{digest}.ext"))
}

fn extension_storage_path(root: &Path, id: &str) -> PathBuf {
    root.join(EXTENSION_STORAGE_DIR).join(format!("{id}.json"))
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), PersistentManagerError> {
    if path.exists() {
        let existing = fs::read(path).map_err(storage_error)?;
        return if existing == bytes {
            Ok(())
        } else {
            Err(storage("immutable package path contains different bytes"))
        };
    }
    atomic_write(path, bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PersistentManagerError> {
    let parent = path.parent().ok_or_else(|| storage("invalid storage path"))?;
    fs::create_dir_all(parent).map_err(storage_error)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temp).map_err(storage_error)?;
        file.write_all(bytes).map_err(storage_error)?;
        file.sync_all().map_err(storage_error)?;
        fs::rename(&temp, path).map_err(storage_error)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn remove_if_unreferenced(path: &Path) {
    let _ = fs::remove_file(path);
}
fn storage(message: &str) -> PersistentManagerError {
    PersistentManagerError::Storage(message.to_owned())
}
fn storage_error(error: std::io::Error) -> PersistentManagerError {
    PersistentManagerError::Storage(error.to_string())
}

#[cfg(test)]
mod extension_storage_tests {
    use super::*;

    #[test]
    fn storage_quota_rejection_preserves_previous_values() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aurora-extension-storage-quota-{}-{stamp}", std::process::id()));
        let store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("open store");
        let id = "dev.example.fixture";
        let original = vec![ExtensionStorageEntry {
            key: "saved".into(),
            value: "keep".into(),
        }];
        store.write_extension_storage(id, original.clone()).expect("write initial values");

        let oversized: Vec<_> = (0..9)
            .map(|index| ExtensionStorageEntry {
                key: format!("value-{index}"),
                value: "x".repeat(aurora_extension_api::MAX_STORAGE_VALUE_BYTES),
            })
            .collect();
        assert!(matches!(
            store.write_extension_storage(id, oversized),
            Err(PersistentManagerError::Storage(message)) if message == "extension storage quota exceeded"
        ));
        assert_eq!(store.read_extension_storage(id).expect("original values remain"), original);
        std::fs::remove_dir_all(root).expect("remove isolated test store");
    }

    #[test]
    fn corrupt_extension_storage_is_rejected_without_rewriting_it() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aurora-extension-storage-corrupt-{}-{stamp}", std::process::id()));
        let store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("open store");
        let id = "dev.example.fixture";
        let path = extension_storage_path(&root, id);
        std::fs::create_dir_all(path.parent().expect("storage directory")).expect("create storage directory");

        let cases = [
            b"not json".to_vec(),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": EXTENSION_STORAGE_VERSION + 1,
                "entries": []
            }))
            .expect("unsupported schema"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": EXTENSION_STORAGE_VERSION,
                "entries": [
                    {"key": "duplicate", "value": "first"},
                    {"key": "duplicate", "value": "second"}
                ]
            }))
            .expect("duplicate keys"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": EXTENSION_STORAGE_VERSION,
                "entries": [{"key": "Bad Key", "value": "value"}]
            }))
            .expect("invalid key"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": EXTENSION_STORAGE_VERSION,
                "entries": [{"key": "valid", "value": "bad\nvalue"}]
            }))
            .expect("invalid value"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": EXTENSION_STORAGE_VERSION,
                "entries": (0..=aurora_extension_api::MAX_STORAGE_ENTRIES)
                    .map(|index| serde_json::json!({"key": format!("key-{index}"), "value": "v"}))
                    .collect::<Vec<_>>()
            }))
            .expect("excess entries"),
        ];

        for bytes in cases {
            std::fs::write(&path, &bytes).expect("write corrupt storage fixture");
            assert!(store.read_extension_storage(id).is_err());
            assert_eq!(std::fs::read(&path).expect("read original corrupt bytes"), bytes);
        }

        let oversized = vec![b' '; (MAX_EXTENSION_STORAGE_BYTES + 1) as usize];
        std::fs::write(&path, &oversized).expect("write oversized storage fixture");
        assert!(store.read_extension_storage(id).is_err());
        assert_eq!(std::fs::read(&path).expect("read original oversized bytes"), oversized);
        std::fs::remove_dir_all(root).expect("remove isolated test store");
    }
}
