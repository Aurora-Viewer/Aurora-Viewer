//! Offline Asset & Mesh Pre-Flight Inspector (`asset_inspect` / `asset_inspect_local`).
//!
//! Parses local `.gltf`, `.glb`, `.dae` (COLLADA), and image textures on disk
//! before uploading or previewing in-world. Computes SHA-256 digests, vertex
//! and triangle counts, material slot names, bounding boxes, and Second Life
//! upload readiness warnings (power-of-two textures, max 8 materials per mesh).

use crate::bridge::McpError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_INSPECT_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const ALLOWED_ASSET_EXTENSIONS: &[&str] = &[
    "gltf", "glb", "dae", "png", "jpg", "jpeg", "tga", "bmp", "webp", "gif",
];

/// Validates that `raw_path` points to a regular file with an allowed extension
/// inside an approved local root (current working directory, `aurora-mcp-captures`,
/// `aurora-live-mesh`, or `AURORA_MCP_ASSET_ROOTS`) and within `max_bytes`.
pub fn validate_local_file_path(
    raw_path: &str,
    allowed_exts: &[&str],
    max_bytes: Option<u64>,
) -> Result<PathBuf, McpError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(McpError::InvalidParams("file_path cannot be empty".into()));
    }
    let canonical = std::fs::canonicalize(Path::new(trimmed))
        .map_err(|e| McpError::InvalidParams(format!("cannot resolve '{trimmed}': {e}")))?;

    let meta = std::fs::metadata(&canonical)
        .map_err(|e| McpError::InvalidParams(format!("cannot stat '{}': {e}", canonical.display())))?;
    if !meta.is_file() {
        return Err(McpError::InvalidParams(format!(
            "'{}' is not a regular file",
            canonical.display()
        )));
    }
    if let Some(limit) = max_bytes
        && meta.len() > limit
    {
        return Err(McpError::InvalidParams(format!(
            "file '{}' ({} bytes) exceeds maximum allowed size ({limit} bytes)",
            canonical.display(),
            meta.len()
        )));
    }

    let ext = canonical
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !allowed_exts.iter().any(|allowed| *allowed == ext) {
        return Err(McpError::PermissionDenied(format!(
            "file extension '.{ext}' is not permitted; allowed: {}",
            allowed_exts.join(", ")
        )));
    }

    let mut allowed_roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir()
        && let Ok(c) = std::fs::canonicalize(cwd)
    {
        allowed_roots.push(c);
    }
    for sub in ["aurora-mcp-captures", "aurora-live-mesh"] {
        let dir = std::env::temp_dir().join(sub);
        if let Ok(c) = std::fs::canonicalize(dir) {
            allowed_roots.push(c);
        }
    }
    if let Ok(env_roots) = std::env::var("AURORA_MCP_ASSET_ROOTS") {
        for entry in std::env::split_paths(&env_roots) {
            if let Ok(c) = std::fs::canonicalize(entry) {
                allowed_roots.push(c);
            }
        }
    }

    if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(McpError::PermissionDenied(format!(
            "path '{}' is outside allowed asset roots (cwd, capture/temp dirs, or AURORA_MCP_ASSET_ROOTS)",
            canonical.display()
        )));
    }

    Ok(canonical)
}

pub fn inspect_local_asset(file_path: &str) -> Result<Value, McpError> {
    let path = validate_local_file_path(
        file_path,
        ALLOWED_ASSET_EXTENSIONS,
        Some(MAX_INSPECT_ASSET_BYTES),
    )?;
    let bytes = std::fs::read(&path)
        .map_err(|e| McpError::InvalidParams(format!("cannot read '{}': {e}", path.display())))?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "gltf" => inspect_gltf_json(&bytes, &path, &sha256, "gltf"),
        "glb" => inspect_glb(&bytes, &path, &sha256),
        "dae" => inspect_collada(&bytes, &path, &sha256),
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "webp" | "gif" => {
            inspect_texture(&bytes, &path, &sha256, &ext)
        }
        _ => Ok(json!({
            "filename": path.display().to_string(),
            "format": ext,
            "size_bytes": bytes.len(),
            "sha256": sha256,
            "warnings": ["unrecognized extension for deep 3D/texture inspection"],
        })),
    }
}

fn inspect_glb(bytes: &[u8], path: &Path, sha256: &str) -> Result<Value, McpError> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err(McpError::InvalidParams("invalid GLB header magic".into()));
    }
    let chunk_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let chunk_type = &bytes[16..20];
    if chunk_type != b"JSON" || bytes.len() < 20 + chunk_len {
        return Err(McpError::InvalidParams("GLB missing valid JSON chunk".into()));
    }
    inspect_gltf_json(&bytes[20..20 + chunk_len], path, sha256, "glb")
}

fn inspect_gltf_json(
    json_bytes: &[u8],
    path: &Path,
    sha256: &str,
    format: &str,
) -> Result<Value, McpError> {
    let doc: Value = serde_json::from_slice(json_bytes)
        .map_err(|e| McpError::InvalidParams(format!("invalid glTF JSON: {e}")))?;

    let accessors = doc
        .get("accessors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let materials: Vec<String> = doc
        .get("materials")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(i, m)| {
                    m.get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Material_{i}"))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut total_vertices = 0u64;
    let mut total_triangles = 0u64;
    let mut bbox_min = [f64::INFINITY; 3];
    let mut bbox_max = [f64::NEG_INFINITY; 3];
    let mut warnings = Vec::new();
    let mut mesh_summaries = Vec::new();

    if let Some(meshes) = doc.get("meshes").and_then(Value::as_array) {
        for (mi, mesh) in meshes.iter().enumerate() {
            let name = mesh
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Mesh")
                .to_owned();
            let primitives = mesh
                .get("primitives")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if primitives.len() > 8 {
                warnings.push(format!(
                    "mesh '{name}' (#{mi}) has {} primitives/material faces; Second Life supports at most 8 faces per prim",
                    primitives.len()
                ));
            }
            let mut mesh_verts = 0u64;
            let mut mesh_tris = 0u64;
            for prim in &primitives {
                if let Some(pos_idx) = prim
                    .get("attributes")
                    .and_then(|a| a.get("POSITION"))
                    .and_then(Value::as_u64)
                    && let Some(acc) = accessors.get(pos_idx as usize)
                {
                    let vcount = acc.get("count").and_then(Value::as_u64).unwrap_or(0);
                    mesh_verts += vcount;
                    if vcount > 65_536 {
                        warnings.push(format!(
                            "mesh '{name}' primitive exceeds 65,536 vertices ({vcount})"
                        ));
                    }
                    if let (Some(min), Some(max)) = (
                        acc.get("min").and_then(Value::as_array),
                        acc.get("max").and_then(Value::as_array),
                    ) && min.len() == 3
                        && max.len() == 3
                    {
                        for axis in 0..3 {
                            if let Some(v) = min[axis].as_f64() {
                                bbox_min[axis] = bbox_min[axis].min(v);
                            }
                            if let Some(v) = max[axis].as_f64() {
                                bbox_max[axis] = bbox_max[axis].max(v);
                            }
                        }
                    }
                }
                if let Some(idx_acc) = prim
                    .get("indices")
                    .and_then(Value::as_u64)
                    .and_then(|i| accessors.get(i as usize))
                {
                    let icount = idx_acc.get("count").and_then(Value::as_u64).unwrap_or(0);
                    mesh_tris += icount / 3;
                }
            }
            total_vertices += mesh_verts;
            total_triangles += mesh_tris;
            mesh_summaries.push(json!({
                "name": name,
                "primitive_faces": primitives.len(),
                "vertices": mesh_verts,
                "triangles": mesh_tris,
            }));
        }
    }

    let dimensions = if bbox_min[0].is_finite() && bbox_max[0].is_finite() {
        json!([
            ((bbox_max[0] - bbox_min[0]) * 1000.0).round() / 1000.0,
            ((bbox_max[1] - bbox_min[1]) * 1000.0).round() / 1000.0,
            ((bbox_max[2] - bbox_min[2]) * 1000.0).round() / 1000.0,
        ])
    } else {
        Value::Null
    };

    Ok(json!({
        "filename": path.display().to_string(),
        "format": format,
        "sha256": sha256,
        "mesh_count": mesh_summaries.len(),
        "meshes": mesh_summaries,
        "total_vertices": total_vertices,
        "total_triangles": total_triangles,
        "material_slots": materials,
        "bounding_box_dimensions": dimensions,
        "warnings": warnings,
    }))
}

fn inspect_collada(bytes: &[u8], path: &Path, sha256: &str) -> Result<Value, McpError> {
    let xml = std::str::from_utf8(bytes)
        .map_err(|e| McpError::InvalidParams(format!("COLLADA file is not UTF-8: {e}")))?;
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| McpError::InvalidParams(format!("invalid COLLADA XML: {e}")))?;

    let mut geometries = Vec::new();
    let mut materials = Vec::new();
    let mut total_triangles = 0u64;
    let mut warnings = Vec::new();

    for node in doc.descendants() {
        match node.tag_name().name() {
            "material" => {
                let name = node
                    .attribute("name")
                    .or_else(|| node.attribute("id"))
                    .unwrap_or("Material");
                materials.push(name.to_owned());
            }
            "geometry" => {
                let geom_name = node
                    .attribute("name")
                    .or_else(|| node.attribute("id"))
                    .unwrap_or("Geometry")
                    .to_owned();
                let mut faces = 0usize;
                let mut tris = 0u64;
                for child in node.descendants() {
                    if matches!(child.tag_name().name(), "triangles" | "polylist" | "polygons") {
                        faces += 1;
                        let count = child
                            .attribute("count")
                            .and_then(|c| c.parse::<u64>().ok())
                            .unwrap_or(0);
                        tris += count;
                    }
                }
                if faces > 8 {
                    warnings.push(format!(
                        "COLLADA geometry '{geom_name}' has {faces} material faces (Second Life max is 8)"
                    ));
                }
                total_triangles += tris;
                geometries.push(json!({
                    "name": geom_name,
                    "material_faces": faces,
                    "triangles": tris,
                }));
            }
            _ => {}
        }
    }

    Ok(json!({
        "filename": path.display().to_string(),
        "format": "dae",
        "size_bytes": bytes.len(),
        "sha256": sha256,
        "geometry_count": geometries.len(),
        "geometries": geometries,
        "total_triangles": total_triangles,
        "material_slots": materials,
        "warnings": warnings,
    }))
}

fn inspect_texture(
    bytes: &[u8],
    path: &Path,
    sha256: &str,
    ext: &str,
) -> Result<Value, McpError> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| McpError::InvalidParams(format!("cannot decode image: {e}")))?;
    let width = img.width();
    let height = img.height();
    let is_pot = width.is_power_of_two() && height.is_power_of_two();
    let mut warnings = Vec::new();
    if !is_pot {
        warnings.push(format!(
            "dimensions {width}x{height} are not powers of two; Second Life will resample on upload"
        ));
    }
    if width > 2048 || height > 2048 {
        warnings.push(format!(
            "dimensions {width}x{height} exceed Second Life maximum texture size (2048x2048)"
        ));
    }

    Ok(json!({
        "filename": path.display().to_string(),
        "format": ext,
        "size_bytes": bytes.len(),
        "sha256": sha256,
        "width": width,
        "height": height,
        "power_of_two": is_pot,
        "color_type": format!("{:?}", img.color()),
        "warnings": warnings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_gltf_inspector_extracts_bounds_and_warnings() {
        let gltf = br#"{
            "asset": { "version": "2.0" },
            "materials": [{ "name": "Hull" }, { "name": "Glass" }],
            "accessors": [
                { "count": 24, "min": [-0.5, -0.5, 0.0], "max": [0.5, 0.5, 2.0] },
                { "count": 36 }
            ],
            "meshes": [{
                "name": "Crate",
                "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "material": 0 }]
            }]
        }"#;
        let res = inspect_gltf_json(gltf, Path::new("crate.gltf"), "deadbeef", "gltf")
            .expect("inspect gltf");
        assert_eq!(res["total_vertices"], 24);
        assert_eq!(res["total_triangles"], 12);
        assert_eq!(res["bounding_box_dimensions"], json!([1.0, 1.0, 2.0]));
        assert_eq!(res["material_slots"], json!(["Hull", "Glass"]));
    }

    #[test]
    fn mcp_asset_inspector_rejects_disallowed_extensions_and_outside_paths() {
        let dir = std::env::temp_dir().join("aurora-mcp-test-outside");
        let _ = std::fs::create_dir_all(&dir);
        let secret_txt = dir.join("secret.txt");
        let _ = std::fs::write(&secret_txt, b"secret");
        assert!(matches!(
            inspect_local_asset(&secret_txt.display().to_string()),
            Err(McpError::PermissionDenied(_))
        ));

        let outside_png = dir.join("outside.png");
        let _ = std::fs::write(&outside_png, b"not-a-png");
        assert!(matches!(
            inspect_local_asset(&outside_png.display().to_string()),
            Err(McpError::PermissionDenied(_))
        ));
    }
}
