//! Ship both matching Blender package layouts inside the viewer, so the bridge
//! needs no source checkout or Python installation outside Blender.
use parking_lot::Mutex;
use std::{
    io::{Cursor, Write},
    sync::Arc,
};

const FILES: &[(&str, &[u8])] = &[
    ("__init__.py", include_bytes!("../../../../blender/aurora_live/__init__.py")),
    ("compat.py", include_bytes!("../../../../blender/aurora_live/compat.py")),
    ("client.py", include_bytes!("../../../../blender/aurora_live/client.py")),
    ("exporter.py", include_bytes!("../../../../blender/aurora_live/exporter.py")),
    ("starter.py", include_bytes!("../../../../blender/aurora_live/starter.py")),
    ("lods.py", include_bytes!("../../../../blender/aurora_live/lods.py")),
    ("shape_apply.py", include_bytes!("../../../../blender/aurora_live/shape_apply.py")),
    (
        "shape_transfer.py",
        include_bytes!("../../../../blender/aurora_live/shape_transfer.py"),
    ),
    ("weights.py", include_bytes!("../../../../blender/aurora_live/weights.py")),
    ("materials.py", include_bytes!("../../../../blender/aurora_live/materials.py")),
    ("tracking.py", include_bytes!("../../../../blender/aurora_live/tracking.py")),
    ("rigging.py", include_bytes!("../../../../blender/aurora_live/rigging.py")),
    ("handles.py", include_bytes!("../../../../blender/aurora_live/handles.py")),
    ("posing.py", include_bytes!("../../../../blender/aurora_live/posing.py")),
    ("skinning.py", include_bytes!("../../../../blender/aurora_live/skinning.py")),
    ("tools_ui.py", include_bytes!("../../../../blender/aurora_live/tools_ui.py")),
    (
        "SKELETON-NOTICE.txt",
        include_bytes!("../../../../blender/aurora_live/SKELETON-NOTICE.txt"),
    ),
    ("avatar_skeleton.xml", include_bytes!("../../assets/character/avatar_skeleton.xml")),
    (
        "blender_manifest.toml",
        include_bytes!("../../../../blender/aurora_live/blender_manifest.toml"),
    ),
    ("LICENSE", include_bytes!("../../../../LICENSE")),
];

pub(super) fn archive(legacy: bool) -> Result<Vec<u8>, String> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in FILES {
        if legacy && *name == "blender_manifest.toml" {
            continue;
        }
        let path = if legacy {
            format!("aurora_live/{name}")
        } else {
            (*name).to_owned()
        };
        writer.start_file(path, options).map_err(|error| error.to_string())?;
        writer.write_all(bytes).map_err(|error| error.to_string())?;
    }
    writer.finish().map(|cursor| cursor.into_inner()).map_err(|error| error.to_string())
}

pub(super) fn version() -> &'static str {
    // The extension manifest is authoritative for both bundled layouts.
    include_str!("../../../../blender/aurora_live/blender_manifest.toml")
        .lines()
        .find_map(|line| line.strip_prefix("version = \"").and_then(|v| v.strip_suffix('"')))
        .unwrap_or("unknown")
}

#[derive(Default)]
pub struct Addon {
    pub legacy: bool,
    pub busy: bool,
    pub saved: bool,
    pub error: Option<String>,
    result: Arc<Mutex<Option<Result<bool, String>>>>,
}

impl Addon {
    pub fn poll(&mut self) {
        if let Some(result) = self.result.lock().take() {
            self.busy = false;
            match result {
                Ok(saved) => self.saved = saved,
                Err(error) => self.error = Some(error),
            }
        }
    }

    pub fn save(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.saved = false;
        self.error = None;
        let result = self.result.clone();
        let ctx = ctx.clone();
        let legacy = self.legacy;
        jobs.execute(move || {
            let saved = (|| {
                let Some(path) = rfd::FileDialog::new()
                    .set_title(aurora_i18n::tr!("blender-live-save-addon"))
                    .set_file_name(if legacy {
                        "aurora-live-mesh-legacy.zip"
                    } else {
                        "aurora-live-mesh.zip"
                    })
                    .add_filter("ZIP", &["zip"])
                    .save_file()
                else {
                    return Ok(false);
                };
                let bytes = archive(legacy)?;
                crate::atomic_file::write(&path, &bytes).map_err(|error| error.to_string())?;
                Ok(true)
            })();
            *result.lock() = Some(saved);
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn blender_live_bundled_packages_contain_matching_sources_in_both_installable_layouts() {
        for legacy in [false, true] {
            let bytes = archive(legacy).expect("archive");
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("valid ZIP");
            assert_eq!(zip.len(), FILES.len() - usize::from(legacy));
            for (name, original) in FILES {
                if legacy && *name == "blender_manifest.toml" {
                    assert!(zip.by_name("aurora_live/blender_manifest.toml").is_err());
                    continue;
                }
                let path = if legacy {
                    format!("aurora_live/{name}")
                } else {
                    (*name).to_owned()
                };
                let mut file = zip.by_name(&path).expect("entry");
                assert!(file.enclosed_name().is_some());
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).expect("source");
                assert_eq!(contents.as_slice(), *original, "matching {path}");
            }
        }
    }
}
