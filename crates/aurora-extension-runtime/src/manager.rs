use crate::{GuestModule, Manifest, PackageError, ValidatedPackage, VerifiedSignature, validate};
use aurora_extension_api::{ExtensionAction, ExtensionEvent, ExtensionRequest, Permission};
use semver::Version;
use std::collections::{BTreeMap, HashSet};

/// The user-visible state of an installed extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionState {
    /// Installed but not enabled.
    Disabled,
    /// Explicitly enabled with all requested permissions granted.
    Enabled,
    /// Disabled after a guest trap, malformed response or capability violation.
    Quarantined,
}

/// Publisher trust is a local user decision, separate from signature validity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublisherTrustStatus {
    /// The package is unsigned or its valid signing key is not locally trusted.
    UnverifiedPlugin { key_id: Option<String> },
    /// The signature is valid and this key was explicitly trusted by the user.
    VerifiedPublisher { key_id: String, label: String },
}

/// A locally trusted publisher key. The display label is a local alias, not a
/// verified legal identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedPublisher {
    pub key_id: String,
    pub public_key: [u8; 32],
    pub label: String,
}

/// Safe metadata shown to the extension manager before installation consent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionReview {
    /// Declared package identity and API compatibility.
    pub manifest: Manifest,
    /// Canonical package-content hash; integrity only, not publisher authentication.
    pub content_sha256: [u8; 32],
    /// Requested capabilities, parsed from the manifest allowlist.
    pub requested_permissions: Vec<Permission>,
    /// Local trust decision for the package signing key, if any.
    pub publisher_trust: PublisherTrustStatus,
    /// Newly requested permissions not in the previous grant set.
    pub added_permissions: Vec<Permission>,
    /// Whether an update changes the signing key associated with this extension ID.
    pub publisher_identity_changed: bool,
    /// Previous signer fingerprint during an update, if any.
    pub previous_signature_key: Option<String>,
    /// Whether the update asks for permissions not previously granted.
    pub permissions_changed: bool,
}

/// A package that has passed validation but has not yet been installed or granted permissions.
pub struct StagedExtension {
    review: ExtensionReview,
    package: ValidatedPackage,
    guest: GuestModule,
}

impl StagedExtension {
    /// Read the package review data for an install or update consent screen.
    pub fn review(&self) -> &ExtensionReview {
        &self.review
    }
}

struct InstalledExtension {
    manifest: Manifest,
    content_sha256: [u8; 32],
    signature: Option<VerifiedSignature>,
    unverified_acknowledged: bool,
    requested: HashSet<Permission>,
    granted: HashSet<Permission>,
    guest: GuestModule,
    state: ExtensionState,
}

/// Safe metadata for one installed extension; never contains guest code or secrets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionSummary {
    /// Parsed extension manifest.
    pub manifest: Manifest,
    /// Integrity hash of the installed package.
    pub content_sha256: [u8; 32],
    /// Current trust status; a valid signature alone does not establish trust.
    pub publisher_trust: PublisherTrustStatus,
    /// Fingerprint of the cryptographically verified signing key, if present.
    pub signature_key_id: Option<String>,
    /// Whether the user explicitly acknowledged an unverified signing key.
    pub unverified_acknowledged: bool,
    /// Requested permissions from the manifest.
    pub requested_permissions: Vec<Permission>,
    /// Permissions explicitly granted by the user.
    pub granted_permissions: Vec<Permission>,
    /// Installed lifecycle state.
    pub state: ExtensionState,
}

/// A lifecycle or dispatch failure. Errors are intentionally bounded and do not contain guest text.
#[derive(Debug)]
pub enum ManagerError {
    /// Package validation or WASM compilation failed.
    Package(PackageError),
    /// The package ID is already installed.
    AlreadyInstalled,
    /// The requested extension ID is not installed.
    NotInstalled,
    /// An attempted approval included an unrequested or duplicate capability.
    InvalidPermissionSet,
    /// Enabling requires explicit grants for every requested permission.
    MissingPermissions,
    /// The permission to revoke was not requested or is not currently granted.
    PermissionNotGranted,
    /// An update must keep the same extension ID and increase its semantic version.
    InvalidUpdate,
    /// Safe mode blocks activation and event execution.
    SafeMode,
    /// The extension is installed but disabled.
    Disabled,
    /// The extension is quarantined and requires explicit recovery.
    Quarantined,
    /// The host supplied a malformed or incompatible event.
    InvalidEvent,
    /// The incoming event needs a capability that has not been granted.
    EventPermissionDenied,
    /// Guest requested an output action without its corresponding grant.
    CapabilityViolation,
    /// Explicit acknowledgement is required before installing an unverified package.
    UnverifiedAcknowledgementRequired,
    /// An update changes publisher keys and requires explicit user review.
    PublisherIdentityChanged,
    /// The guest trapped, exhausted fuel or produced an invalid API response.
    RuntimeFailure,
}

impl std::fmt::Display for ManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Package(error) => return write!(f, "{error}"),
            Self::AlreadyInstalled => "extension is already installed",
            Self::NotInstalled => "extension is not installed",
            Self::InvalidPermissionSet => "permission approval is invalid",
            Self::MissingPermissions => "all requested permissions require explicit approval",
            Self::PermissionNotGranted => "permission is not currently granted",
            Self::InvalidUpdate => "extension update is invalid",
            Self::SafeMode => "extension execution is disabled in safe mode",
            Self::Disabled => "extension is disabled",
            Self::Quarantined => "extension is quarantined",
            Self::InvalidEvent => "host event is invalid",
            Self::EventPermissionDenied => "event permission was not granted",
            Self::CapabilityViolation => "extension requested an ungranted capability",
            Self::UnverifiedAcknowledgementRequired => "unverified extension requires explicit user acknowledgement",
            Self::PublisherIdentityChanged => "extension publisher key changed and requires explicit acknowledgement",
            Self::RuntimeFailure => "extension failed and was quarantined",
        })
    }
}

impl std::error::Error for ManagerError {}

impl From<PackageError> for ManagerError {
    fn from(error: PackageError) -> Self {
        Self::Package(error)
    }
}

/// In-memory lifecycle and permission broker. The viewer persists packages and grants separately.
pub struct ExtensionManager {
    supported_api: Version,
    safe_mode: bool,
    trusted_publishers: BTreeMap<String, String>,
    extensions: BTreeMap<String, InstalledExtension>,
}

impl ExtensionManager {
    /// Create a manager for one supported API version; extensions start disabled and ungranted.
    pub fn new(supported_api: Version) -> Self {
        Self {
            supported_api,
            safe_mode: false,
            trusted_publishers: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }

    /// Replace the manager's locally trusted key-fingerprint labels. Removing trust
    /// disables an unacknowledged signed extension; it never enables one.
    pub(crate) fn set_trusted_publishers(&mut self, publishers: &[TrustedPublisher]) {
        self.trusted_publishers = publishers
            .iter()
            .map(|publisher| (publisher.key_id.clone(), publisher.label.clone()))
            .collect();
        let trusted_keys: HashSet<_> = self.trusted_publishers.keys().cloned().collect();
        for extension in self.extensions.values_mut() {
            let still_trusted = extension
                .signature
                .as_ref()
                .is_some_and(|signature| trusted_keys.contains(&signature.key_id));
            if !still_trusted && !extension.unverified_acknowledged && extension.state == ExtensionState::Enabled {
                extension.state = ExtensionState::Disabled;
            }
        }
    }

    /// Explicitly revoking a key invalidates prior unverified acknowledgements
    /// for every package signed by it and disables those extensions.
    pub(crate) fn revoke_publisher(&mut self, key_id: &str) {
        for extension in self.extensions.values_mut() {
            if extension.signature.as_ref().is_some_and(|signature| signature.key_id == key_id) {
                extension.unverified_acknowledged = false;
                if extension.state == ExtensionState::Enabled {
                    extension.state = ExtensionState::Disabled;
                }
            }
        }
    }

    /// Enable or disable host-wide safe mode. This never changes stored user grants.
    pub fn set_safe_mode(&mut self, enabled: bool) {
        self.safe_mode = enabled;
    }

    /// Whether all third-party extension dispatch is currently blocked.
    pub fn safe_mode(&self) -> bool {
        self.safe_mode
    }

    /// Validate and stage a new package without installing or granting it.
    pub fn stage_install(&self, bytes: &[u8]) -> Result<StagedExtension, ManagerError> {
        let staged = self.stage_package(bytes)?;
        if self.extensions.contains_key(&staged.review.manifest.id) {
            return Err(ManagerError::AlreadyInstalled);
        }
        Ok(staged)
    }

    /// Validate a package as an update for an existing ID; no state changes until `apply_update`.
    pub fn stage_update(&self, id: &str, bytes: &[u8]) -> Result<StagedExtension, ManagerError> {
        let current = self.extensions.get(id).ok_or(ManagerError::NotInstalled)?;
        let mut staged = self.stage_package(bytes)?;
        if staged.review.manifest.id != id {
            return Err(ManagerError::InvalidUpdate);
        }
        let old_version = Version::parse(&current.manifest.version).map_err(|_| ManagerError::InvalidUpdate)?;
        let new_version = Version::parse(&staged.review.manifest.version).map_err(|_| ManagerError::InvalidUpdate)?;
        if new_version <= old_version {
            return Err(ManagerError::InvalidUpdate);
        }
        let previous_key = current.signature.as_ref().map(|signature| signature.key_id.clone());
        let next_key = staged.package.signature.as_ref().map(|signature| signature.key_id.clone());
        staged.review.publisher_identity_changed = previous_key != next_key;
        staged.review.previous_signature_key = previous_key;
        staged.review.added_permissions = staged
            .review
            .requested_permissions
            .iter()
            .copied()
            .filter(|permission| !current.granted.contains(permission))
            .collect();
        staged.review.permissions_changed = !staged.review.added_permissions.is_empty();
        Ok(staged)
    }

    fn stage_package(&self, bytes: &[u8]) -> Result<StagedExtension, ManagerError> {
        let package = validate(bytes, &self.supported_api)?;
        let guest = GuestModule::compile(&package.module)?;
        let requested_permissions = parse_permissions(&package.manifest)?;
        let review = ExtensionReview {
            manifest: package.manifest.clone(),
            content_sha256: package.content_sha256,
            requested_permissions,
            publisher_trust: self.publisher_trust(package.signature.as_ref()),
            added_permissions: Vec::new(),
            publisher_identity_changed: false,
            previous_signature_key: None,
            permissions_changed: false,
        };
        Ok(StagedExtension { review, package, guest })
    }

    /// Install using the safe defaults: an unverified package or publisher-key change is refused.
    pub fn install(
        &mut self,
        staged: StagedExtension,
        approved_permissions: &[Permission],
        enable: bool,
    ) -> Result<ExtensionSummary, ManagerError> {
        self.install_with_ack(staged, approved_permissions, enable, false, false)
    }

    /// Install a reviewed package after explicit trust and permission consent.
    pub fn install_with_ack(
        &mut self,
        staged: StagedExtension,
        approved_permissions: &[Permission],
        enable: bool,
        acknowledge_unverified: bool,
        acknowledge_publisher_change: bool,
    ) -> Result<ExtensionSummary, ManagerError> {
        if self.extensions.contains_key(&staged.review.manifest.id) {
            return Err(ManagerError::AlreadyInstalled);
        }
        if matches!(&staged.review.publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. }) && enable && !acknowledge_unverified {
            return Err(ManagerError::UnverifiedAcknowledgementRequired);
        }
        if staged.review.publisher_identity_changed && !acknowledge_publisher_change {
            return Err(ManagerError::PublisherIdentityChanged);
        }
        if enable && self.safe_mode {
            return Err(ManagerError::SafeMode);
        }
        let granted = approved_set(&staged.review.requested_permissions, approved_permissions)?;
        if enable && granted != requested_set(&staged.review.requested_permissions) {
            return Err(ManagerError::MissingPermissions);
        }
        let state = if enable {
            ExtensionState::Enabled
        } else {
            ExtensionState::Disabled
        };
        let id = staged.package.manifest.id.clone();
        self.extensions.insert(
            id.clone(),
            InstalledExtension {
                manifest: staged.package.manifest,
                content_sha256: staged.package.content_sha256,
                signature: staged.package.signature,
                unverified_acknowledged: acknowledge_unverified,
                requested: requested_set(&staged.review.requested_permissions),
                granted,
                guest: staged.guest,
                state,
            },
        );
        self.summary(&id)
    }

    /// Apply an update using safe defaults: new permission requests and publisher changes need review.
    pub fn apply_update(
        &mut self,
        staged: StagedExtension,
        approved_permissions: &[Permission],
        enable: bool,
    ) -> Result<ExtensionSummary, ManagerError> {
        self.apply_update_with_ack(staged, approved_permissions, enable, false, false)
    }

    /// Apply an update only after explicit permission and publisher-identity consent.
    pub fn apply_update_with_ack(
        &mut self,
        staged: StagedExtension,
        approved_permissions: &[Permission],
        enable: bool,
        acknowledge_unverified: bool,
        acknowledge_publisher_change: bool,
    ) -> Result<ExtensionSummary, ManagerError> {
        let id = staged.review.manifest.id.clone();
        let current = self.extensions.get(&id).ok_or(ManagerError::NotInstalled)?;
        let current_version = Version::parse(&current.manifest.version).map_err(|_| ManagerError::InvalidUpdate)?;
        let next_version = Version::parse(&staged.package.manifest.version).map_err(|_| ManagerError::InvalidUpdate)?;
        if next_version <= current_version {
            return Err(ManagerError::InvalidUpdate);
        }
        if matches!(&staged.review.publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. }) && enable && !acknowledge_unverified {
            return Err(ManagerError::UnverifiedAcknowledgementRequired);
        }
        if staged.review.publisher_identity_changed && !acknowledge_publisher_change {
            return Err(ManagerError::PublisherIdentityChanged);
        }
        if enable && self.safe_mode {
            return Err(ManagerError::SafeMode);
        }
        let granted = approved_set(&staged.review.requested_permissions, approved_permissions)?;
        if enable && granted != requested_set(&staged.review.requested_permissions) {
            return Err(ManagerError::MissingPermissions);
        }
        let state = if enable {
            ExtensionState::Enabled
        } else {
            ExtensionState::Disabled
        };
        self.extensions.insert(
            id.clone(),
            InstalledExtension {
                manifest: staged.package.manifest,
                content_sha256: staged.package.content_sha256,
                signature: staged.package.signature,
                unverified_acknowledged: acknowledge_unverified,
                requested: requested_set(&staged.review.requested_permissions),
                granted,
                guest: staged.guest,
                state,
            },
        );
        self.summary(&id)
    }

    /// Enable an installed extension after verifying the full, explicit grant set.
    pub fn enable(&mut self, id: &str, approved_permissions: &[Permission]) -> Result<(), ManagerError> {
        self.enable_with_ack(id, approved_permissions, false)
    }

    /// Enable an unverified extension only after explicit acknowledgement and full grant approval.
    pub fn enable_with_ack(
        &mut self,
        id: &str,
        approved_permissions: &[Permission],
        acknowledge_unverified: bool,
    ) -> Result<(), ManagerError> {
        if self.safe_mode {
            return Err(ManagerError::SafeMode);
        }
        let publisher_trust = {
            let extension = self.extensions.get(id).ok_or(ManagerError::NotInstalled)?;
            self.publisher_trust(extension.signature.as_ref())
        };
        let extension = self.extensions.get_mut(id).ok_or(ManagerError::NotInstalled)?;
        if extension.state == ExtensionState::Quarantined {
            return Err(ManagerError::Quarantined);
        }
        if matches!(publisher_trust, PublisherTrustStatus::UnverifiedPlugin { .. })
            && !extension.unverified_acknowledged
            && !acknowledge_unverified
        {
            return Err(ManagerError::UnverifiedAcknowledgementRequired);
        }
        let requested = permission_vec(&extension.requested);
        let granted = approved_set(&requested, approved_permissions)?;
        if granted != extension.requested {
            return Err(ManagerError::MissingPermissions);
        }
        extension.granted = granted;
        extension.unverified_acknowledged |= acknowledge_unverified;
        extension.state = ExtensionState::Enabled;
        Ok(())
    }

    /// Disable an extension without changing its explicitly granted permissions.
    pub fn disable(&mut self, id: &str) -> Result<(), ManagerError> {
        let extension = self.extensions.get_mut(id).ok_or(ManagerError::NotInstalled)?;
        if extension.state != ExtensionState::Quarantined {
            extension.state = ExtensionState::Disabled;
        }
        Ok(())
    }

    /// Revoke one grant. Any revocation disables the extension until all required grants are approved again.
    pub fn revoke_permission(&mut self, id: &str, permission: Permission) -> Result<(), ManagerError> {
        let extension = self.extensions.get_mut(id).ok_or(ManagerError::NotInstalled)?;
        if !extension.requested.contains(&permission) || !extension.granted.remove(&permission) {
            return Err(ManagerError::PermissionNotGranted);
        }
        extension.state = ExtensionState::Disabled;
        Ok(())
    }

    /// Clear quarantine after user review. This leaves the extension disabled; enabling still requires consent.
    pub fn clear_quarantine(&mut self, id: &str) -> Result<(), ManagerError> {
        let extension = self.extensions.get_mut(id).ok_or(ManagerError::NotInstalled)?;
        if extension.state == ExtensionState::Quarantined {
            extension.state = ExtensionState::Disabled;
        }
        Ok(())
    }

    /// Restore a durable quarantine marker from the local registry during startup.
    pub(crate) fn restore_quarantine(&mut self, id: &str) -> Result<(), ManagerError> {
        let extension = self.extensions.get_mut(id).ok_or(ManagerError::NotInstalled)?;
        extension.state = ExtensionState::Quarantined;
        Ok(())
    }

    /// Remove an extension and all in-memory grants. The viewer removes its package files separately.
    pub fn uninstall(&mut self, id: &str) -> Result<(), ManagerError> {
        self.extensions.remove(id).map(|_| ()).ok_or(ManagerError::NotInstalled)
    }

    /// Return safe metadata for every installed extension.
    pub fn list(&self) -> Vec<ExtensionSummary> {
        self.extensions.keys().filter_map(|id| self.summary(id).ok()).collect()
    }

    /// Return safe metadata for one installed extension.
    pub fn summary(&self, id: &str) -> Result<ExtensionSummary, ManagerError> {
        let extension = self.extensions.get(id).ok_or(ManagerError::NotInstalled)?;
        Ok(ExtensionSummary {
            manifest: extension.manifest.clone(),
            content_sha256: extension.content_sha256,
            publisher_trust: self.publisher_trust(extension.signature.as_ref()),
            signature_key_id: extension.signature.as_ref().map(|signature| signature.key_id.clone()),
            unverified_acknowledged: extension.unverified_acknowledged,
            requested_permissions: permission_vec(&extension.requested),
            granted_permissions: permission_vec(&extension.granted),
            state: extension.state,
        })
    }

    fn publisher_trust(&self, signature: Option<&VerifiedSignature>) -> PublisherTrustStatus {
        let Some(signature) = signature else {
            return PublisherTrustStatus::UnverifiedPlugin { key_id: None };
        };
        match self.trusted_publishers.get(&signature.key_id) {
            Some(label) => PublisherTrustStatus::VerifiedPublisher {
                key_id: signature.key_id.clone(),
                label: label.clone(),
            },
            None => PublisherTrustStatus::UnverifiedPlugin {
                key_id: Some(signature.key_id.clone()),
            },
        }
    }

    /// Deliver one event only when enabled and all required input/output grants are present.
    pub fn dispatch(&mut self, id: &str, request: &ExtensionRequest) -> Result<Vec<ExtensionAction>, ManagerError> {
        if self.safe_mode {
            return Err(ManagerError::SafeMode);
        }
        request.validate().map_err(|_| ManagerError::InvalidEvent)?;
        let extension = self.extensions.get_mut(id).ok_or(ManagerError::NotInstalled)?;
        match extension.state {
            ExtensionState::Disabled => return Err(ManagerError::Disabled),
            ExtensionState::Quarantined => return Err(ManagerError::Quarantined),
            ExtensionState::Enabled => {}
        }
        if let Some(permission) = event_permission(&request.event)
            && !extension.granted.contains(&permission)
        {
            return Err(ManagerError::EventPermissionDenied);
        }
        let response = match extension.guest.handle_event(request) {
            Ok(response) => response,
            Err(_) => {
                extension.state = ExtensionState::Quarantined;
                return Err(ManagerError::RuntimeFailure);
            }
        };
        for action in &response.actions {
            if !extension.granted.contains(&action_permission(action)) {
                extension.state = ExtensionState::Quarantined;
                return Err(ManagerError::CapabilityViolation);
            }
        }
        Ok(response.actions)
    }
}

fn parse_permissions(manifest: &Manifest) -> Result<Vec<Permission>, ManagerError> {
    manifest
        .permissions
        .iter()
        .map(|key| Permission::from_manifest_key(key).ok_or(ManagerError::InvalidPermissionSet))
        .collect()
}

fn permission_vec(permissions: &HashSet<Permission>) -> Vec<Permission> {
    let mut sorted: Vec<_> = permissions.iter().copied().collect();
    sorted.sort_by_key(|permission| permission.manifest_key());
    sorted
}

fn requested_set(permissions: &[Permission]) -> HashSet<Permission> {
    permissions.iter().copied().collect()
}

fn approved_set(requested: &[Permission], approved: &[Permission]) -> Result<HashSet<Permission>, ManagerError> {
    let requested = requested_set(requested);
    let approved_set: HashSet<_> = approved.iter().copied().collect();
    if approved_set.len() != approved.len() || !approved_set.is_subset(&requested) {
        return Err(ManagerError::InvalidPermissionSet);
    }
    Ok(approved_set)
}

fn event_permission(event: &ExtensionEvent) -> Option<Permission> {
    match event {
        ExtensionEvent::NearbyChat { .. } => Some(Permission::ChatNearbyRead),
        ExtensionEvent::PanelAction { .. } => Some(Permission::UiPanelRegister),
        ExtensionEvent::Initialize { .. } | ExtensionEvent::Shutdown => None,
    }
}

fn action_permission(action: &ExtensionAction) -> Permission {
    match action {
        ExtensionAction::AnnotateChat { .. } => Permission::UiChatAnnotate,
        ExtensionAction::ShowNotification { .. } => Permission::UiNotificationShow,
        ExtensionAction::RegisterPanel { .. } => Permission::UiPanelRegister,
    }
}
