//! Defensive `.ext` archive validation and import-free WASM execution for the Aurora extension platform.
//!
//! This crate validates packages, runs a versioned event protocol with bounded resources,
//! and deliberately exposes no WASI, filesystem, network or viewer-object imports.

use aurora_extension_api::{ExtensionRequest, ExtensionResponse, Permission};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read},
};
use wasmi::{Config, EnforcedLimits, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use zip::ZipArchive;

mod manager;
mod signature;
mod store;
pub use manager::*;
pub use signature::{
    MAX_SIGNATURE_BYTES, PublisherKeyFile, SIGNATURE_MEMBER, SignatureEnvelope, SignatureError, VerifiedSignature, encode_envelope,
    generate_pkcs8, parse_public_key_file, public_key_file, public_key_from_pkcs8, sign_manifest,
};
pub use store::*;

pub const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_EOCD_SEARCH_BYTES: usize = 22 + u16::MAX as usize;
const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_PATH_DEPTH: usize = 8;
const MAX_RATIO: u64 = 100;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    pub entry: String,
    pub permissions: Vec<String>,
    #[serde(default)]
    pub network_domains: Vec<String>,
    pub publisher: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedPackage {
    pub manifest: Manifest,
    pub module: Vec<u8>,
    /// SHA-256 over sorted package content, excluding the detached signature envelope.
    pub content_sha256: [u8; 32],
    /// Cryptographically valid signer, if the archive carries a signature. Trust is user-managed separately.
    pub signature: Option<VerifiedSignature>,
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageError(pub String);

impl PackageError {
    /// Return the reason when validation deliberately blocked a package for a security failure.
    pub fn blocked_reason(&self) -> Option<&str> {
        self.0.strip_prefix("blocked plugin: ")
    }
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PackageError {}
fn invalid(message: impl Into<String>) -> PackageError {
    PackageError(message.into())
}
fn blocked(message: impl Into<String>) -> PackageError {
    PackageError(format!("blocked plugin: {}", message.into()))
}

const MAX_GUEST_MEMORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_GUEST_TABLE_ELEMENTS: usize = 1024;
const VALIDATION_FUEL: u64 = 1_000_000;
const MAX_MESSAGE_BYTES: usize = aurora_extension_api::MAX_ENVELOPE_BYTES;
const MAX_RESPONSE_BYTES: usize = aurora_extension_api::MAX_ENVELOPE_BYTES;

fn sandbox_engine() -> Engine {
    let mut config = Config::default();
    config
        .allow_start_fn(false)
        .consume_fuel(true)
        .enforced_limits(EnforcedLimits::strict())
        .set_max_recursion_depth(128)
        .set_max_stack_height(1024 * 1024)
        .set_max_cached_stacks(0);
    Engine::new(&config)
}

fn sandbox_store(engine: &Engine) -> Result<Store<StoreLimits>, PackageError> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_GUEST_MEMORY_BYTES)
        .table_elements(MAX_GUEST_TABLE_ELEMENTS)
        .instances(1)
        .tables(1)
        .memories(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(engine, limits);
    store.limiter(|limits| limits);
    store
        .set_fuel(VALIDATION_FUEL)
        .map_err(|_| invalid("could not configure WebAssembly fuel"))?;
    Ok(store)
}

/// A compiled, import-free v1 module. Reuse this object for multiple calls; each
/// transform gets a fresh store so guest memory and fuel are not carried over.
pub struct GuestModule {
    engine: Engine,
    module: Module,
}

impl GuestModule {
    /// Compile and instantiate a module while rejecting imports and missing ABI exports.
    pub fn compile(module_bytes: &[u8]) -> Result<Self, PackageError> {
        let engine = sandbox_engine();
        let module = Module::new(&engine, module_bytes).map_err(|_| invalid("invalid or unsupported WebAssembly module"))?;
        if module.imports().next().is_some() {
            return Err(invalid("WebAssembly imports are not enabled"));
        }
        let mut store = sandbox_store(&engine)?;
        let instance = Linker::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .map_err(|_| invalid("WebAssembly instantiation failed"))?;
        instance
            .get_memory(&store, "memory")
            .ok_or_else(|| invalid("module does not export memory"))?;
        instance
            .get_typed_func::<i32, i32>(&store, "alloc")
            .map_err(|_| invalid("module does not export the v1 allocator"))?;
        instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&store, "handle")
            .map_err(|_| invalid("module does not export the v1 handler function"))?;
        Ok(Self { engine, module })
    }

    /// Run one bounded UTF-8 request/response call with fresh guest memory and fuel.
    pub fn invoke(&self, input: &str) -> Result<String, PackageError> {
        if input.len() > MAX_MESSAGE_BYTES {
            return Err(invalid("request exceeds byte limit"));
        }
        let mut store = sandbox_store(&self.engine)?;
        let instance = Linker::new(&self.engine)
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|_| invalid("WebAssembly instantiation failed"))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| invalid("module does not export memory"))?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, "alloc")
            .map_err(|_| invalid("module does not export the v1 allocator"))?;
        let handler = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&store, "handle")
            .map_err(|_| invalid("module does not export the v1 handler function"))?;

        let input_len = i32::try_from(input.len()).map_err(|_| invalid("input length is invalid"))?;
        let output_capacity = i32::try_from(MAX_RESPONSE_BYTES).map_err(|_| invalid("output limit is invalid"))?;
        let input_ptr = alloc
            .call(&mut store, input_len)
            .map_err(|_| invalid("module allocation trapped"))?;
        let output_ptr = alloc
            .call(&mut store, output_capacity)
            .map_err(|_| invalid("module allocation trapped"))?;
        let input_start = usize::try_from(input_ptr).map_err(|_| invalid("module returned an invalid input pointer"))?;
        let output_start = usize::try_from(output_ptr).map_err(|_| invalid("module returned an invalid output pointer"))?;
        let input_end = input_start
            .checked_add(input.len())
            .ok_or_else(|| invalid("input range overflow"))?;
        let output_end = output_start
            .checked_add(MAX_RESPONSE_BYTES)
            .ok_or_else(|| invalid("output range overflow"))?;
        let memory_size = memory.data_size(&store);
        if input_end > memory_size || output_end > memory_size {
            return Err(invalid("module allocation is outside linear memory"));
        }
        if input_start < output_end && output_start < input_end {
            return Err(invalid("module allocations overlap"));
        }
        memory
            .write(&mut store, input_start, input.as_bytes())
            .map_err(|_| invalid("could not write input to module memory"))?;
        let output_len = handler
            .call(&mut store, (input_ptr, input_len, output_ptr, output_capacity))
            .map_err(|_| invalid("module execution trapped or exhausted fuel"))?;
        let output_len = usize::try_from(output_len).map_err(|_| invalid("module returned an invalid output length"))?;
        if output_len > MAX_RESPONSE_BYTES {
            return Err(invalid("module output exceeds byte limit"));
        }
        let mut output = vec![0; output_len];
        memory
            .read(&store, output_start, &mut output)
            .map_err(|_| invalid("could not read module output"))?;
        let output = String::from_utf8(output).map_err(|_| invalid("module output is not valid UTF-8"))?;
        if output.chars().any(char::is_control) {
            return Err(invalid("module output contains control characters"));
        }
        Ok(output)
    }

    /// Send one typed lifecycle/event request through the guest API and validate its response.
    pub fn handle_event(&self, request: &ExtensionRequest) -> Result<ExtensionResponse, PackageError> {
        request.validate().map_err(|_| invalid("invalid extension API request"))?;
        let input = serde_json::to_string(request).map_err(|_| invalid("could not serialize extension API request"))?;
        let output = self.invoke(&input)?;
        let response: ExtensionResponse = serde_json::from_str(&output).map_err(|_| invalid("extension response is malformed"))?;
        response
            .validate_for(request)
            .map_err(|_| invalid("extension response violates the API contract"))?;
        Ok(response)
    }
}

/// Validate a module without retaining the compiled engine.
pub fn validate_wasm(module_bytes: &[u8]) -> Result<(), PackageError> {
    GuestModule::compile(module_bytes).map(|_| ())
}

/// Compile and run one one-shot v1 request. Repeated callers should retain a `GuestModule`.
pub fn invoke(module_bytes: &[u8], input: &str) -> Result<String, PackageError> {
    GuestModule::compile(module_bytes)?.invoke(input)
}

fn preflight_zip_entry_count(bytes: &[u8]) -> Result<usize, PackageError> {
    const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";
    if bytes.len() < 22 {
        return Err(invalid("invalid ZIP archive"));
    }
    let first_candidate = bytes.len().saturating_sub(MAX_EOCD_SEARCH_BYTES);
    let last_candidate = bytes.len() - 22;
    let eocd = (first_candidate..=last_candidate)
        .rev()
        .find(|&offset| {
            if bytes.get(offset..offset + 4) != Some(&EOCD_SIGNATURE[..]) {
                return false;
            }
            let comment_len = u16::from_le_bytes([bytes[offset + 20], bytes[offset + 21]]) as usize;
            offset + 22 + comment_len == bytes.len()
        })
        .ok_or_else(|| invalid("invalid ZIP end-of-central-directory record"))?;

    let disk_number = u16::from_le_bytes([bytes[eocd + 4], bytes[eocd + 5]]);
    let central_directory_disk = u16::from_le_bytes([bytes[eocd + 6], bytes[eocd + 7]]);
    let disk_entries = u16::from_le_bytes([bytes[eocd + 8], bytes[eocd + 9]]);
    let total_entries = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]);
    let central_directory_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().expect("bounded EOCD"));
    let central_directory_offset = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().expect("bounded EOCD"));
    let has_zip64_locator = eocd >= 20 && bytes.get(eocd - 20..eocd - 16) == Some(&ZIP64_LOCATOR_SIGNATURE[..]);
    if total_entries == u16::MAX || central_directory_size == u32::MAX || central_directory_offset == u32::MAX || has_zip64_locator {
        return Err(invalid("Zip64 archives are not supported"));
    }
    if disk_number != 0 || central_directory_disk != 0 || disk_entries != total_entries {
        return Err(invalid("multi-disk ZIP archives are not supported"));
    }
    if total_entries as usize > MAX_FILES {
        return Err(invalid("invalid archive entry count"));
    }
    Ok(total_entries as usize)
}

/// Parse and validate an entire `.ext` ZIP without writing archive paths to disk.
pub fn validate(bytes: &[u8], supported_api: &Version) -> Result<ValidatedPackage, PackageError> {
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(invalid("archive exceeds byte limit"));
    }
    let expected_entry_count = preflight_zip_entry_count(bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("invalid ZIP archive"))?;
    if archive.is_empty() || archive.len() > MAX_FILES || archive.len() != expected_entry_count {
        return Err(invalid("invalid archive entry count"));
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut names = HashSet::new();
    let mut path_spellings = BTreeMap::<String, String>::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| invalid("invalid ZIP entry"))?;
        let raw_name = entry.name().map_err(|_| invalid("invalid ZIP entry name"))?.into_owned();
        if !raw_name.is_ascii() {
            return Err(invalid("archive paths must be ASCII"));
        }
        validate_path(raw_name.as_ref(), entry.is_dir())?;
        let normalized_name = raw_name.trim_end_matches('/');
        let mut prefix = String::new();
        for component in normalized_name.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let canonical = prefix.to_ascii_lowercase();
            if path_spellings.get(&canonical).is_some_and(|spelling| spelling != &prefix) {
                return Err(invalid("case-colliding archive path components"));
            }
            path_spellings.insert(canonical, prefix.clone());
        }
        if !names.insert(normalized_name.to_ascii_lowercase()) {
            return Err(invalid("duplicate or case-colliding archive path"));
        }
        if entry.is_symlink()
            || entry.unix_mode().is_some_and(|mode| {
                let kind = mode & 0o170000;
                kind != 0 && kind != 0o100000 && kind != 0o040000
            })
        {
            return Err(invalid("symlinks and special files are not allowed"));
        }
        if entry.is_dir() {
            if entry.size() != 0 || entry.compressed_size() != 0 {
                return Err(invalid("directory entries may not contain payload data"));
            }
            continue;
        }
        let declared = entry.size();
        if raw_name == SIGNATURE_MEMBER && declared > MAX_SIGNATURE_BYTES as u64 {
            return Err(invalid("signature envelope exceeds byte limit"));
        }
        if declared > MAX_FILE_BYTES {
            return Err(invalid("archive member exceeds byte limit"));
        }
        let compressed = entry.compressed_size();
        if declared > 0 && (compressed == 0 || declared > compressed.saturating_mul(MAX_RATIO)) {
            return Err(invalid("archive member compression ratio exceeds limit"));
        }
        expanded = expanded.checked_add(declared).ok_or_else(|| invalid("expanded size overflow"))?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(invalid("expanded archive exceeds byte limit"));
        }
        let mut contents = Vec::with_capacity(declared as usize);
        entry
            .take(declared + 1)
            .read_to_end(&mut contents)
            .map_err(|_| invalid("could not decompress archive member"))?;
        if contents.len() as u64 != declared {
            return Err(invalid("archive member size mismatch"));
        }
        files.insert(raw_name, contents);
    }
    let file_paths: Vec<String> = files.keys().map(|path| path.to_ascii_lowercase()).collect();
    for (index, path) in file_paths.iter().enumerate() {
        if file_paths
            .iter()
            .enumerate()
            .any(|(other_index, other)| other_index != index && other.starts_with(&format!("{path}/")))
        {
            return Err(invalid("archive file conflicts with a directory path"));
        }
    }
    let manifest_bytes = files.get("manifest.json").ok_or_else(|| invalid("manifest.json is missing"))?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("manifest exceeds byte limit"));
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| invalid("manifest is malformed or has unsupported fields"))?;
    validate_manifest(&manifest, supported_api)?;
    let module = files.get(&manifest.entry).ok_or_else(|| invalid("entry module is missing"))?;
    if module.len() < 8 || &module[..4] != b"\0asm" || module[4..8] != [1, 0, 0, 0] {
        return Err(invalid("entry is not a supported WebAssembly module"));
    }
    for path in files.keys() {
        if path != "manifest.json" && path != &manifest.entry && path != SIGNATURE_MEMBER && !path.starts_with("assets/") {
            return Err(invalid("only the entry WASM, signature.json and assets/ files are permitted"));
        }
    }
    let mut hash = Sha256::new();
    for (path, content) in &files {
        if path == SIGNATURE_MEMBER {
            continue;
        }
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update((content.len() as u64).to_le_bytes());
        hash.update(content);
    }
    let content_sha256 = hash.finalize().into();
    let signature = files
        .get(SIGNATURE_MEMBER)
        .map(|bytes| signature::verify_envelope(&manifest, &content_sha256, bytes).map_err(|error| blocked(error.0)))
        .transpose()?;
    validate_wasm(module)?;
    let module = files.remove(&manifest.entry).ok_or_else(|| invalid("entry module is missing"))?;
    files.remove("manifest.json");
    files.remove(SIGNATURE_MEMBER);
    Ok(ValidatedPackage {
        manifest,
        module,
        content_sha256,
        signature,
        assets: files,
    })
}

fn validate_path(path: &str, is_dir: bool) -> Result<(), PackageError> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') || path.contains('\0') {
        return Err(invalid("unsafe archive path"));
    }
    let name = if is_dir { path.strip_suffix('/').unwrap_or(path) } else { path };
    let parts: Vec<_> = name.split('/').collect();
    if parts.is_empty()
        || parts.len() > MAX_PATH_DEPTH
        || parts.iter().any(|p| {
            let stem = p
                .split('.')
                .next()
                .unwrap_or_default()
                .trim_end_matches([' ', '.'])
                .to_ascii_uppercase();
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.ends_with(['.', ' '])
                || p.contains([':', '<', '>', '"', '|', '?', '*'])
                || p.chars().any(char::is_control)
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
                || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
    {
        return Err(invalid("unsafe archive path"));
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest, supported_api: &Version) -> Result<(), PackageError> {
    if manifest.manifest_version != 1 {
        return Err(invalid("unsupported manifest version"));
    }
    if manifest.id.is_empty()
        || manifest.id.len() > 128
        || !manifest.id.is_ascii()
        || manifest.id.split('.').any(|part| {
            part.is_empty()
                || !part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || !part.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                || !part.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        })
    {
        return Err(invalid("invalid extension id"));
    }
    if manifest.name.trim().is_empty()
        || manifest.name.len() > 128
        || manifest.publisher.trim().is_empty()
        || manifest.publisher.len() > 128
        || manifest.name.chars().any(char::is_control)
        || manifest.publisher.chars().any(char::is_control)
    {
        return Err(invalid("invalid extension identity"));
    }
    Version::parse(&manifest.version).map_err(|_| invalid("invalid extension version"))?;
    let api = VersionReq::parse(&manifest.api).map_err(|_| invalid("invalid API compatibility range"))?;
    if !api.matches(supported_api) {
        return Err(invalid("extension API version is incompatible"));
    }
    if manifest.entry != "module.wasm" {
        return Err(invalid("entry must be module.wasm"));
    }
    let mut requested = HashSet::new();
    for permission in &manifest.permissions {
        if Permission::from_manifest_key(permission).is_none() || !requested.insert(permission) {
            return Err(invalid("unknown or duplicate permission"));
        }
    }
    if !manifest.network_domains.is_empty() {
        return Err(invalid("network permissions are not supported by this package runtime yet"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_extension_api::{API_VERSION, ExtensionAction, ExtensionEvent};
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for (name, contents) in entries {
            writer.start_file(*name, options).expect("archive entry");
            writer.write_all(contents).expect("archive bytes");
        }
        writer.finish().expect("finish archive").into_inner()
    }
    fn push_uleb(output: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }
    fn push_sleb(output: &mut Vec<u8>, mut value: i32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
            if !done {
                byte |= 0x80;
            }
            output.push(byte);
            if done {
                break;
            }
        }
    }

    fn wasm_section(module: &mut Vec<u8>, id: u8, payload: &[u8]) {
        module.push(id);
        push_uleb(module, payload.len() as u32);
        module.extend_from_slice(payload);
    }
    fn handler_module(handler_body: &[u8]) -> Vec<u8> {
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        wasm_section(
            &mut module,
            1,
            &[2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7f],
        );
        wasm_section(&mut module, 3, &[2, 0, 1]);
        wasm_section(&mut module, 5, &[1, 0, 1]);
        wasm_section(&mut module, 6, &[1, 0x7f, 1, 0x41, 0x80, 0x08, 0x0b]);
        wasm_section(
            &mut module,
            7,
            &[
                3, 6, b'm', b'e', b'm', b'o', b'r', b'y', 2, 0, 5, b'a', b'l', b'l', b'o', b'c', 0, 0, 6, b'h', b'a', b'n', b'd', b'l',
                b'e', 0, 1,
            ],
        );
        let allocate = [0, 0x23, 0, 0x20, 0, 0x23, 0, 0x6a, 0x24, 0, 0x0b];
        let mut code = vec![2];
        push_uleb(&mut code, allocate.len() as u32);
        code.extend_from_slice(&allocate);
        push_uleb(&mut code, handler_body.len() as u32);
        code.extend_from_slice(handler_body);
        wasm_section(&mut module, 10, &code);
        module
    }

    fn copy_module() -> Vec<u8> {
        handler_module(&[0, 0x20, 2, 0x20, 0, 0x20, 1, 0xfc, 0x0a, 0, 0, 0x20, 1, 0x0b])
    }

    fn looping_module() -> Vec<u8> {
        handler_module(&[0, 0x03, 0x40, 0x0c, 0, 0x0b, 0x00, 0x0b])
    }

    fn response_module(response: &[u8]) -> Vec<u8> {
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
        let mut handler = vec![0, 0x20, 2, 0x41];
        push_sleb(&mut handler, 64);
        handler.push(0x41);
        push_sleb(&mut handler, i32::try_from(response.len()).expect("bounded fixture"));
        handler.extend_from_slice(&[0xfc, 0x0a, 0, 0, 0x41]);
        push_sleb(&mut handler, i32::try_from(response.len()).expect("bounded fixture"));
        handler.push(0x0b);
        let mut code = vec![2];
        for body in [allocate.as_slice(), handler.as_slice()] {
            push_uleb(&mut code, body.len() as u32);
            code.extend_from_slice(body);
        }
        wasm_section(&mut module, 10, &code);
        let mut data = vec![1, 0, 0x41];
        push_sleb(&mut data, 64);
        data.push(0x0b);
        push_uleb(&mut data, response.len() as u32);
        data.extend_from_slice(response);
        wasm_section(&mut module, 11, &data);
        module
    }

    #[test]
    fn import_free_guest_can_invoke_and_preserves_utf8() {
        let wasm = copy_module();
        assert_eq!(invoke(&wasm, "bonjour 🌻").expect("bounded transform"), "bonjour 🌻");
        assert!(invoke(&wasm, "line\nbreak").is_err());
    }

    #[test]
    fn minimal_package_fixture_validates_and_runs_the_generic_api() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 1,
            event: aurora_extension_api::ExtensionEvent::NearbyChat {
                message_id: 7,
                speaker: "Fixture".into(),
                text: "sample".into(),
            },
        };
        let expected = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 1,
            actions: vec![aurora_extension_api::ExtensionAction::AnnotateChat {
                message_id: 7,
                text: "fixture response".into(),
            }],
        };
        let wasm = response_module(&serde_json::to_vec(&expected).expect("fixture response JSON"));
        let bytes = fixture(&manifest(">=1.0.0, <2.0.0", "[\"chat.nearby.read\",\"ui.chat.annotate\"]"), &wasm);
        let package = validate(&bytes, &Version::new(1, 0, 0)).expect("minimal fixture package");
        assert_eq!(package.manifest.id, "dev.example.fixture");
        let guest = GuestModule::compile(&package.module).expect("compile minimal fixture");
        assert_eq!(guest.handle_event(&request).expect("fixture event response"), expected);
        assert!(package.manifest.network_domains.is_empty());
    }

    #[test]
    fn extension_manager_requires_consent_and_honors_safe_mode_and_revoke() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 1,
            event: ExtensionEvent::NearbyChat {
                message_id: 7,
                speaker: "Fixture".into(),
                text: "sample".into(),
            },
        };
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 1,
            actions: vec![ExtensionAction::AnnotateChat {
                message_id: 7,
                text: "fixture response".into(),
            }],
        };
        let wasm = response_module(&serde_json::to_vec(&response).expect("fixture response JSON"));
        let bytes = fixture(&manifest(">=1.0.0, <2.0.0", "[\"chat.nearby.read\",\"ui.chat.annotate\"]"), &wasm);
        let id = "dev.example.fixture";

        let mut manager = ExtensionManager::new(Version::new(1, 0, 0));
        let staged = manager.stage_install(&bytes).expect("stage package");
        let installed = manager
            .install_with_ack(staged, &[], false, true, false)
            .expect("install disabled without grants");
        assert_eq!(installed.state, ExtensionState::Disabled);
        assert!(manager.dispatch(id, &request).is_err());
        assert!(matches!(
            manager.enable(id, &[Permission::ChatNearbyRead]),
            Err(ManagerError::MissingPermissions)
        ));
        assert!(manager.summary(id).expect("still installed").granted_permissions.is_empty());
        manager
            .enable(id, &[Permission::ChatNearbyRead, Permission::UiChatAnnotate])
            .expect("explicit consent");
        assert_eq!(manager.dispatch(id, &request).expect("authorized event"), response.actions);

        manager.set_safe_mode(true);
        assert!(matches!(manager.dispatch(id, &request), Err(ManagerError::SafeMode)));
        manager.set_safe_mode(false);
        manager.revoke_permission(id, Permission::UiChatAnnotate).expect("revoke grant");
        assert_eq!(manager.summary(id).expect("revoked").state, ExtensionState::Disabled);
        assert!(matches!(
            manager.enable(id, &[Permission::ChatNearbyRead]),
            Err(ManagerError::MissingPermissions)
        ));
    }

    #[test]
    fn extension_manager_quarantines_capability_violations_and_runtime_traps() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 1,
            event: ExtensionEvent::NearbyChat {
                message_id: 7,
                speaker: "Fixture".into(),
                text: "sample".into(),
            },
        };
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 1,
            actions: vec![ExtensionAction::AnnotateChat {
                message_id: 7,
                text: "fixture response".into(),
            }],
        };
        let wasm = response_module(&serde_json::to_vec(&response).expect("fixture response JSON"));
        let bytes = fixture(&manifest(">=1.0.0, <2.0.0", "[\"chat.nearby.read\"]"), &wasm);
        let id = "dev.example.fixture";
        let mut manager = ExtensionManager::new(Version::new(1, 0, 0));
        let staged = manager.stage_install(&bytes).expect("stage package");
        manager
            .install_with_ack(staged, &[Permission::ChatNearbyRead], true, true, false)
            .expect("enable input-only fixture");
        assert!(matches!(manager.dispatch(id, &request), Err(ManagerError::CapabilityViolation)));
        assert_eq!(manager.summary(id).expect("quarantined").state, ExtensionState::Quarantined);
        manager.clear_quarantine(id).expect("clear quarantine after review");
        assert_eq!(
            manager.summary(id).expect("disabled after recovery").state,
            ExtensionState::Disabled
        );

        let mut trap_manifest = manifest("*", "[]");
        trap_manifest = trap_manifest.replace("dev.example.fixture", "dev.example.loop");
        let trap_bytes = fixture(&trap_manifest, &looping_module());
        let staged = manager.stage_install(&trap_bytes).expect("stage looping fixture");
        manager
            .install_with_ack(staged, &[], true, true, false)
            .expect("enable no-permission fixture");
        let init = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 2,
            event: ExtensionEvent::Initialize {
                host_api_version: "1.0.0".into(),
            },
        };
        assert!(matches!(
            manager.dispatch("dev.example.loop", &init),
            Err(ManagerError::RuntimeFailure)
        ));
        assert_eq!(
            manager.summary("dev.example.loop").expect("trap quarantined").state,
            ExtensionState::Quarantined
        );
    }

    #[test]
    fn extension_updates_require_explicit_reapproval_of_added_permissions() {
        let wasm = copy_module();
        let initial = fixture(&manifest("*", "[\"chat.nearby.read\"]"), &wasm);
        let id = "dev.example.fixture";
        let mut manager = ExtensionManager::new(Version::new(1, 0, 0));
        let staged = manager.stage_install(&initial).expect("stage v0.1.0");
        manager
            .install_with_ack(staged, &[Permission::ChatNearbyRead], true, true, false)
            .expect("enable initial package");

        let mut update_manifest = manifest("*", "[\"chat.nearby.read\",\"ui.notification.show\"]");
        update_manifest = update_manifest.replace("\"version\":\"0.1.0\"", "\"version\":\"0.2.0\"");
        let update = fixture(&update_manifest, &wasm);
        let staged = manager.stage_update(id, &update).expect("stage permission-adding update");
        assert!(staged.review().permissions_changed);
        assert_eq!(staged.review().added_permissions, vec![Permission::UiNotificationShow]);
        assert!(matches!(
            manager.apply_update_with_ack(staged, &[Permission::ChatNearbyRead], true, true, false),
            Err(ManagerError::MissingPermissions)
        ));
        assert_eq!(manager.summary(id).expect("old version retained").manifest.version, "0.1.0");

        let staged = manager.stage_update(id, &update).expect("stage again for explicit approval");
        manager
            .apply_update_with_ack(
                staged,
                &[Permission::ChatNearbyRead, Permission::UiNotificationShow],
                true,
                true,
                false,
            )
            .expect("approve update explicitly");
        assert_eq!(manager.summary(id).expect("updated").manifest.version, "0.2.0");
    }

    #[test]
    fn guest_fuel_stops_an_infinite_loop() {
        let wasm = looping_module();
        assert!(invoke(&wasm, "hello").is_err());
    }

    #[test]
    fn oversized_guest_input_is_rejected_before_execution() {
        let wasm = copy_module();
        let input = "a".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(invoke(&wasm, &input).is_err());
    }

    fn fixture(manifest: &str, wasm: &[u8]) -> Vec<u8> {
        archive(&[("manifest.json", manifest.as_bytes()), ("module.wasm", wasm)])
    }
    fn signed_fixture(manifest_json: &str, wasm: &[u8], private_key: &[u8]) -> Vec<u8> {
        let unsigned_bytes = fixture(manifest_json, wasm);
        let package = validate(&unsigned_bytes, &Version::new(1, 0, 0)).expect("unsigned fixture");
        let envelope = sign_manifest(&package.manifest, &package.content_sha256, private_key).expect("sign fixture");
        let signature = encode_envelope(&envelope).expect("signature envelope");
        archive(&[
            ("manifest.json", manifest_json.as_bytes()),
            ("module.wasm", wasm),
            (SIGNATURE_MEMBER, &signature),
        ])
    }
    fn manifest(api: &str, permissions: &str) -> String {
        format!(
            r#"{{"manifest_version":1,"id":"dev.example.fixture","name":"Extension Fixture","version":"0.1.0","api":"{api}","entry":"module.wasm","permissions":{permissions},"network_domains":[],"publisher":"Aurora test fixture"}}"#
        )
    }
    #[test]
    fn wasm_validation_denies_imports_and_oversized_initial_memory() {
        let imported = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x02\x07\x01\x01e\x01f\0\0";
        assert!(validate_wasm(imported).is_err());

        // One 257-page memory exceeds the 16 MiB per-instance cap.
        let large_memory = b"\0asm\x01\0\0\0\x05\x04\x01\0\x81\x02";
        assert!(validate_wasm(large_memory).is_err());
    }

    #[test]
    fn wasm_validation_rejects_a_start_function() {
        let start = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0\x08\x01\0\x0a\x04\x01\x02\0\x0b";
        assert!(validate_wasm(start).is_err());
    }

    #[test]
    fn accepts_valid_offline_wasm_extension() {
        let manifest_json = manifest(">=1.0.0, <2.0.0", "[\"chat.nearby.read\",\"ui.chat.annotate\"]");
        let wasm = copy_module();
        let bytes = archive(&[
            ("manifest.json", manifest_json.as_bytes()),
            ("module.wasm", &wasm),
            ("assets/label.txt", b"offline sample"),
        ]);
        let package = validate(&bytes, &Version::new(1, 0, 3)).expect("valid package");
        assert_eq!(package.manifest.id, "dev.example.fixture");
        assert_eq!(
            package.assets.get("assets/label.txt").map(Vec::as_slice),
            Some(b"offline sample".as_slice())
        );
        assert_ne!(package.content_sha256, [0; 32]);
    }

    #[test]
    fn signed_package_verifies_and_tampered_content_is_blocked_before_wasm_validation() {
        let manifest_json = manifest("*", "[]");
        let wasm = copy_module();
        let unsigned_bytes = fixture(&manifest_json, &wasm);
        let unsigned = validate(&unsigned_bytes, &Version::new(1, 0, 0)).expect("unsigned package");
        assert!(unsigned.signature.is_none());

        let private_key = generate_pkcs8().expect("signing key");
        let signer = public_key_from_pkcs8(&private_key).expect("public key");
        let envelope = sign_manifest(&unsigned.manifest, &unsigned.content_sha256, &private_key).expect("signature");
        let signature = encode_envelope(&envelope).expect("signature JSON");
        let signed_bytes = archive(&[
            ("manifest.json", manifest_json.as_bytes()),
            ("module.wasm", &wasm),
            (SIGNATURE_MEMBER, &signature),
        ]);
        let signed = validate(&signed_bytes, &Version::new(1, 0, 0)).expect("signed package");
        assert_eq!(signed.content_sha256, unsigned.content_sha256);
        assert_eq!(signed.signature.as_ref().expect("verified signature"), &signer);

        let changed_manifest = manifest_json.replace("Aurora test fixture", "Changed publisher claim");
        let tampered = archive(&[
            ("manifest.json", changed_manifest.as_bytes()),
            ("module.wasm", &wasm),
            (SIGNATURE_MEMBER, &signature),
        ]);
        let error = validate(&tampered, &Version::new(1, 0, 0)).expect_err("tampering must be blocked");
        assert!(error.0.starts_with("blocked plugin:"), "{error}");

        let mut invalid_envelope: serde_json::Value = serde_json::from_slice(&signature).expect("signature JSON");
        let mut invalid_signature = invalid_envelope["signature"].as_str().expect("signature hex").to_owned();
        invalid_signature.replace_range(0..1, if invalid_signature.starts_with('0') { "1" } else { "0" });
        invalid_envelope["signature"] = invalid_signature.into();
        let invalid_signature = serde_json::to_vec(&invalid_envelope).expect("invalid signature JSON");
        let invalid_package = archive(&[
            ("manifest.json", manifest_json.as_bytes()),
            ("module.wasm", &wasm),
            (SIGNATURE_MEMBER, &invalid_signature),
        ]);
        let error = validate(&invalid_package, &Version::new(1, 0, 0)).expect_err("invalid signature must be blocked");
        assert!(error.0.starts_with("blocked plugin:"), "{error}");
    }

    #[test]
    fn unsigned_install_requires_ack_and_local_key_trust_persists_and_revokes() {
        let temp = std::env::temp_dir().join(format!(
            "aurora-extension-trust-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let manifest_json = manifest("*", "[]");
        let wasm = copy_module();
        let unsigned = fixture(&manifest_json, &wasm);
        let private_key = generate_pkcs8().expect("publisher key");
        let signer = public_key_from_pkcs8(&private_key).expect("publisher public key");
        let signed = signed_fixture(&manifest_json, &wasm, &private_key);

        let mut store = PersistentExtensionManager::open(&temp, Version::new(1, 0, 0)).expect("open store");
        let unsigned_staged = store.stage_install(&unsigned).expect("stage unsigned package");
        assert!(matches!(
            &unsigned_staged.review().publisher_trust,
            crate::PublisherTrustStatus::UnverifiedPlugin { key_id: None }
        ));
        assert!(matches!(
            store.install_with_ack(unsigned_staged, &unsigned, &[], true, false, false),
            Err(PersistentManagerError::Manager(ManagerError::UnverifiedAcknowledgementRequired))
        ));

        let key_file = serde_json::to_vec(&public_key_file(&signer.public_key)).expect("publisher public key JSON");
        store.trust_publisher(&key_file, "Fixture publisher").expect("explicitly trust key");
        let staged = store.stage_install(&signed).expect("stage signed package");
        assert_eq!(
            staged.review().publisher_trust,
            crate::PublisherTrustStatus::VerifiedPublisher {
                key_id: signer.key_id.clone(),
                label: "Fixture publisher".into(),
            }
        );
        store
            .install_with_ack(staged, &signed, &[], true, false, false)
            .expect("install trusted package");
        drop(store);

        let mut store = PersistentExtensionManager::open(&temp, Version::new(1, 0, 0)).expect("restore trusted store");
        assert!(matches!(
            store.list()[0].publisher_trust,
            crate::PublisherTrustStatus::VerifiedPublisher { .. }
        ));
        store.untrust_publisher(&signer.key_id).expect("revoke local trust");
        let summaries = store.list();
        let summary = &summaries[0];
        assert_eq!(summary.state, ExtensionState::Disabled);
        assert!(matches!(
            &summary.publisher_trust,
            crate::PublisherTrustStatus::UnverifiedPlugin { .. }
        ));
        assert!(matches!(
            store.enable("dev.example.fixture", &[]),
            Err(PersistentManagerError::Manager(ManagerError::UnverifiedAcknowledgementRequired))
        ));
        store
            .enable_with_ack("dev.example.fixture", &[], true)
            .expect("explicitly acknowledge the now-unverified signer");

        let mut changed_manifest = manifest_json.replace("\"version\":\"0.1.0\"", "\"version\":\"0.2.0\"");
        changed_manifest = changed_manifest.replace("\"publisher\":\"Aurora test fixture\"", "\"publisher\":\"Rotated publisher key\"");
        let rotated_private_key = generate_pkcs8().expect("rotated publisher key");
        let rotated_package = signed_fixture(&changed_manifest, &wasm, &rotated_private_key);
        let staged_update = store
            .stage_update("dev.example.fixture", &rotated_package)
            .expect("stage signer change");
        assert!(staged_update.review().publisher_identity_changed);
        assert!(!staged_update.review().permissions_changed);
        assert!(matches!(
            store.update_with_ack("dev.example.fixture", staged_update, &rotated_package, &[], true, true, false),
            Err(PersistentManagerError::Manager(ManagerError::PublisherIdentityChanged))
        ));
        let staged_update = store
            .stage_update("dev.example.fixture", &rotated_package)
            .expect("restage signer change");
        store
            .update_with_ack("dev.example.fixture", staged_update, &rotated_package, &[], true, true, true)
            .expect("accept signer rotation explicitly");
        drop(store);

        let mut restored = PersistentExtensionManager::open(&temp, Version::new(1, 0, 0)).expect("restore acknowledged store");
        assert_eq!(restored.list()[0].state, ExtensionState::Enabled);
        assert!(restored.list()[0].unverified_acknowledged);
        assert_eq!(restored.list()[0].manifest.version, "0.2.0");
        assert!(matches!(
            restored.list()[0].publisher_trust,
            crate::PublisherTrustStatus::UnverifiedPlugin { .. }
        ));

        let rotated_signer = public_key_from_pkcs8(&rotated_private_key).expect("rotated signer identity");
        let rotated_key_file = serde_json::to_vec(&public_key_file(&rotated_signer.public_key)).expect("rotated public key JSON");
        restored
            .trust_publisher(&rotated_key_file, "Rotated publisher")
            .expect("trust the rotated key");
        assert!(matches!(
            restored.list()[0].publisher_trust,
            crate::PublisherTrustStatus::VerifiedPublisher { .. }
        ));
        restored
            .untrust_publisher(&rotated_signer.key_id)
            .expect("explicitly revoke the rotated key");
        let revoked = &restored.list()[0];
        assert_eq!(revoked.state, ExtensionState::Disabled);
        assert!(!revoked.unverified_acknowledged, "revocation invalidates an older acknowledgement");
        assert!(matches!(
            restored.enable("dev.example.fixture", &[]),
            Err(PersistentManagerError::Manager(ManagerError::UnverifiedAcknowledgementRequired))
        ));
        restored
            .enable_with_ack("dev.example.fixture", &[], true)
            .expect("freshly acknowledge the now-unverified signer");
        assert_eq!(restored.list()[0].state, ExtensionState::Enabled);
        assert!(restored.list()[0].unverified_acknowledged);
        drop(restored);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn rejects_incompatible_api_and_unknown_permissions() {
        let version = Version::new(1, 0, 0);
        assert!(validate(&fixture(&manifest(">=2.0.0", "[]"), b"\0asm\x01\0\0\0"), &version).is_err());
        assert!(validate(&fixture(&manifest("*", "[\"filesystem.read\"]"), b"\0asm\x01\0\0\0"), &version).is_err());
    }
    #[test]
    fn rejects_malformed_module_and_unknown_manifest_fields() {
        let version = Version::new(1, 0, 0);
        assert!(validate(&fixture(&manifest("*", "[]"), b"not wasm"), &version).is_err());
        let extra = manifest("*", "[]").replace("\"publisher\":\"Example\"", "\"publisher\":\"Example\",\"surprise\":true");
        assert!(validate(&fixture(&extra, b"\0asm\x01\0\0\0"), &version).is_err());
    }
    #[test]
    fn rejects_traversal_and_windows_path_aliases() {
        for path in [
            "../module.wasm",
            "C:/module.wasm",
            "folder\\module.wasm",
            "assets/CON.png",
            "assets/CON .txt",
            "assets/NUL.txt",
            "assets/COM1.dat",
            "assets/LPT1 .bin",
            "assets/trailing.",
        ] {
            assert!(validate_path(path, false).is_err(), "{path}");
        }
    }
    #[test]
    fn rejects_zip_symlink_entries() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().unix_permissions(0o120777);
        writer.start_file("assets/link", options).expect("symlink entry");
        writer.write_all(b"target").expect("symlink payload");
        let bytes = writer.finish().expect("archive").into_inner();
        assert!(validate(&bytes, &Version::new(1, 0, 0)).is_err());
    }

    #[test]
    fn rejects_members_with_excessive_compression_ratio() {
        use zip::CompressionMethod;
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file("assets/repeat", options).expect("member");
        writer.write_all(&vec![0; 1024 * 1024]).expect("contents");
        let bytes = writer.finish().expect("archive").into_inner();
        assert!(validate(&bytes, &Version::new(1, 0, 0)).is_err());
    }

    #[test]
    fn enforces_aggregate_expanded_archive_limit() {
        use zip::CompressionMethod;
        let mut seed = 0x1234_5678_u32;
        let pattern: Vec<u8> = (0..16 * 1024)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for file in 0..65 {
            writer.start_file(format!("assets/{file:02}.bin"), options).expect("member");
            for _ in 0..64 {
                writer.write_all(&pattern).expect("contents");
            }
        }
        let bytes = writer.finish().expect("archive").into_inner();
        assert!(bytes.len() < MAX_PACKAGE_BYTES, "fixture must fit the compressed-input cap");
        assert_eq!(
            validate(&bytes, &Version::new(1, 0, 0)).unwrap_err().0,
            "expanded archive exceeds byte limit"
        );
    }

    #[test]
    fn rejects_excessive_zip_entry_count_during_preflight() {
        let mut bytes = archive(&[("assets/ok", b"x")]);
        let eocd = bytes.windows(4).rposition(|magic| magic == b"PK\x05\x06").expect("EOCD");
        bytes[eocd + 8..eocd + 10].copy_from_slice(&129_u16.to_le_bytes());
        bytes[eocd + 10..eocd + 12].copy_from_slice(&129_u16.to_le_bytes());
        assert_eq!(
            validate(&bytes, &Version::new(1, 0, 0)).unwrap_err().0,
            "invalid archive entry count"
        );
    }

    #[test]
    fn rejects_zip64_archives_before_parsing_the_central_directory() {
        let mut bytes = archive(&[("assets/ok", b"x")]);
        let eocd = bytes.windows(4).rposition(|magic| magic == b"PK\x05\x06").expect("EOCD");
        bytes[eocd + 10..eocd + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            validate(&bytes, &Version::new(1, 0, 0)).unwrap_err().0,
            "Zip64 archives are not supported"
        );
    }

    #[test]
    fn rejects_case_aliases_and_traversal_at_archive_boundary() {
        let json = manifest("*", "[]");
        let alias = archive(&[
            ("manifest.json", json.as_bytes()),
            ("MANIFEST.JSON", json.as_bytes()),
            ("module.wasm", b"\0asm\x01\0\0\0"),
        ]);
        let version = Version::new(1, 0, 0);
        assert!(validate(&alias, &version).is_err());
        let traversal = archive(&[("../escape", b"x")]);
        assert!(validate(&traversal, &version).is_err());
    }

    #[test]
    fn rejects_file_directory_prefix_conflicts() {
        let json = manifest("*", "[]");
        let bytes = archive(&[
            ("manifest.json", json.as_bytes()),
            ("module.wasm", b"\0asm\x01\0\0\0"),
            ("assets/a", b"file"),
            ("assets/a/child", b"child"),
        ]);
        assert!(validate(&bytes, &Version::new(1, 0, 0)).is_err());
    }

    #[test]
    fn rejects_oversized_compressed_input_before_parsing() {
        let bytes = vec![0; MAX_PACKAGE_BYTES + 1];
        assert_eq!(
            validate(&bytes, &Version::new(1, 0, 0)).unwrap_err().0,
            "archive exceeds byte limit"
        );
    }

    #[test]
    fn rejects_case_colliding_parent_directories() {
        let json = manifest("*", "[]");
        let bytes = archive(&[
            ("manifest.json", json.as_bytes()),
            ("module.wasm", b"\0asm\x01\0\0\0"),
            ("assets/a.txt", b"a"),
            ("Assets/b.txt", b"b"),
        ]);
        assert!(validate(&bytes, &Version::new(1, 0, 0)).is_err());
    }

    #[test]
    fn persistent_update_requires_new_grants_and_survives_restart() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aurora-extension-update-{}-{stamp}", std::process::id()));
        let id = "dev.example.fixture";
        let original = fixture(&manifest("*", "[\"chat.nearby.read\"]"), &copy_module());
        let updated_manifest =
            manifest("*", "[\"chat.nearby.read\",\"ui.notification.show\"]").replace("\"version\":\"0.1.0\"", "\"version\":\"0.2.0\"");
        let updated = fixture(&updated_manifest, &copy_module());

        let mut store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("open store");
        let staged = store.stage_install(&original).expect("stage original");
        store
            .install_with_ack(staged, &original, &[Permission::ChatNearbyRead], true, true, false)
            .expect("install original with explicit permission");
        let staged_update = store.stage_update(id, &updated).expect("stage newer package");
        assert!(matches!(
            store.update_with_ack(id, staged_update, &updated, &[Permission::ChatNearbyRead], true, true, false),
            Err(PersistentManagerError::Manager(ManagerError::MissingPermissions))
        ));

        let staged_update = store.stage_update(id, &updated).expect("restage after rejected consent");
        let summary = store
            .update_with_ack(
                id,
                staged_update,
                &updated,
                &[Permission::ChatNearbyRead, Permission::UiNotificationShow],
                true,
                true,
                false,
            )
            .expect("update after explicit reapproval");
        assert_eq!(summary.manifest.version, "0.2.0");
        assert_eq!(summary.state, ExtensionState::Enabled);
        drop(store);

        let store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("restore updated package");
        let restored = store.list();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].manifest.version, "0.2.0");
        assert_eq!(restored[0].granted_permissions.len(), 2);
        assert_eq!(restored[0].state, ExtensionState::Enabled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_manager_restores_packages_grants_safe_mode_and_uninstall() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aurora-extension-store-{}-{stamp}", std::process::id()));
        let bytes = fixture(&manifest("*", "[\"chat.nearby.read\"]"), &copy_module());
        let id = "dev.example.fixture";

        let mut store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("open store");
        let staged = store.stage_install(&bytes).expect("stage package");
        let installed = store
            .install_with_ack(staged, &bytes, &[], false, true, false)
            .expect("install without grants");
        assert_eq!(installed.state, ExtensionState::Disabled);
        drop(store);

        let mut store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("restore store");
        let restored = store.list();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].manifest.id, id);
        assert!(restored[0].granted_permissions.is_empty());
        assert_eq!(restored[0].state, ExtensionState::Disabled);
        store.enable(id, &[Permission::ChatNearbyRead]).expect("persist explicit grant");
        store.set_safe_mode(true).expect("persist safe mode");
        drop(store);

        let mut store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("restore enabled store");
        assert!(store.safe_mode());
        assert_eq!(store.list()[0].state, ExtensionState::Enabled);
        store.set_safe_mode(false).expect("disable safe mode");
        store.revoke_permission(id, Permission::ChatNearbyRead).expect("persist revocation");
        assert_eq!(store.list()[0].state, ExtensionState::Disabled);
        assert!(store.list()[0].granted_permissions.is_empty());
        store.uninstall(id).expect("uninstall");
        drop(store);

        let store = PersistentExtensionManager::open(&root, Version::new(1, 0, 0)).expect("restore uninstalled store");
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
