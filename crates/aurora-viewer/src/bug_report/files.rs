//! Blocking diagnostics/image/file work. Called only by the report worker.

use super::{Field, Report};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_IMAGE_FILE: u64 = 16 * 1024 * 1024;
const MAX_LOG_TAIL: u64 = 256 * 1024;

pub struct Diagnostics {
    pub fields: Vec<Field>,
    pub excerpt: String,
}

pub fn diagnostics(demo: bool) -> Diagnostics {
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::everything()),
    );
    let fields = vec![
        Field::new(
            "bug-os",
            "Operating system",
            format!(
                "{} ({})",
                System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()),
                std::env::consts::ARCH
            ),
            true,
        ),
        Field::new(
            "bug-cpu",
            "Processor",
            system.cpus().first().map(|c| c.brand()).unwrap_or("Unavailable"),
            true,
        ),
        Field::new(
            "bug-memory",
            "System memory",
            format!("{} MiB", system.total_memory() / 1024 / 1024),
            true,
        ),
    ];
    let log = if demo {
        "[demo WARN aurora_net] timeout https://example.invalid/cap/private Alice Resident\n[demo ERROR wgpu_core] Validation Error: private label".into()
    } else {
        log_tail()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    Diagnostics {
        fields,
        excerpt: crate::crash::privacy::excerpt(&log),
    }
}

fn log_tail() -> std::io::Result<Vec<u8>> {
    log_tail_from(&crate::logging::log_path())
}

fn log_tail_from(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(MAX_LOG_TAIL)))?;
    let mut bytes = Vec::new();
    file.take(MAX_LOG_TAIL).read_to_end(&mut bytes)?;
    // If the tail began halfway through a record, discard that fragment.
    if len > MAX_LOG_TAIL
        && let Some(end) = bytes.iter().position(|&b| b == b'\n')
    {
        bytes.drain(..=end);
    }
    Ok(bytes)
}

/// Public diagnostics use the same closed vocabulary as live reports. The
/// full native/Rust traces stay local or in the explicitly opted-in ZIP.
pub fn preserved_diagnostics(dir: &Path) -> Diagnostics {
    let mut log = log_tail_from(&dir.join("session.log"))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    for name in ["panic.txt", "native.txt", "failure.txt"] {
        if let Ok(bytes) = log_tail_from(&dir.join(name))
            && !bytes.is_empty()
        {
            log.push_str("\n[crash ERROR aurora_viewer] ");
            log.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    Diagnostics {
        fields: Vec::new(),
        excerpt: crate::crash::privacy::excerpt(&log),
    }
}

#[derive(Clone)]
pub struct Attachment {
    pub png: Vec<u8>,
    pub preview: egui::ColorImage,
    pub width: u32,
    pub height: u32,
}

pub fn choose_image() -> Result<Option<Attachment>, &'static str> {
    rfd::FileDialog::new()
        .set_title(aurora_i18n::tr!("bug-choose-image"))
        .add_filter(aurora_i18n::tr!("bug-images"), &["png", "jpg", "jpeg", "bmp", "tga"])
        .pick_file()
        .map(|path| read_image(&path))
        .transpose()
}

fn read_image(path: &Path) -> Result<Attachment, &'static str> {
    let file = std::fs::File::open(path).map_err(|_| "bug-image-error")?;
    if file.metadata().map_err(|_| "bug-image-error")?.len() > MAX_IMAGE_FILE {
        return Err("bug-image-limit");
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "bug-image-error")?;
    if bytes.len() as u64 > MAX_IMAGE_FILE {
        return Err("bug-image-limit");
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "bug-image-error")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| "bug-image-error")?;
    prepare_image(img)
}

pub fn capture(width: u32, height: u32, pixels: Vec<u8>) -> Result<Attachment, &'static str> {
    let img = image::RgbaImage::from_raw(width, height, pixels).ok_or("bug-image-error")?;
    prepare_image(image::DynamicImage::ImageRgba8(img))
}

fn prepare_image(img: image::DynamicImage) -> Result<Attachment, &'static str> {
    // Re-encode pixel data, never original EXIF, source path or filename.
    let img = if img.width() > 1920 || img.height() > 1920 {
        img.resize(1920, 1920, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let preview = if img.width() > 560 || img.height() > 280 {
        img.thumbnail(560, 280)
    } else {
        img.clone()
    }
    .to_rgba8();
    let preview = egui::ColorImage::from_rgba_unmultiplied([preview.width() as usize, preview.height() as usize], preview.as_raw());
    let (width, height) = (img.width(), img.height());
    let mut png = Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).map_err(|_| "bug-image-error")?;
    Ok(Attachment {
        png: png.into_inner(),
        preview,
        width,
        height,
    })
}

pub struct Saved {
    pub public_dir: PathBuf,
    pub private_log_saved: bool,
}

pub fn save(
    report: &Report,
    image: Option<&Attachment>,
    include_image: bool,
    demo: bool,
    session_dir: Option<&Path>,
) -> std::io::Result<Saved> {
    let root = crate::logging::logs_dir().parent().unwrap_or(Path::new(".")).join("bug-reports");
    let log = session_dir
        .map(|dir| dir.join("session.log"))
        .or_else(|| (!demo).then(crate::logging::log_path));
    save_in(&root, report, image, include_image, log.as_deref(), session_dir)
}

fn save_in(
    root: &Path,
    report: &Report,
    image: Option<&Attachment>,
    include_image: bool,
    private_log: Option<&Path>,
    session_dir: Option<&Path>,
) -> std::io::Result<Saved> {
    let directory = root.join(uuid::Uuid::new_v4().to_string());
    let public = directory.join("public");
    let private = directory.join("private-do-not-upload");
    std::fs::create_dir_all(&public)?;
    std::fs::create_dir_all(&private)?;
    std::fs::write(public.join("report.md"), format!("# {}\n\n{}", report.title, report.markdown()))?;
    if let Some(image) = image {
        std::fs::write(private.join("screenshot.png"), &image.png)?;
        if include_image {
            std::fs::write(public.join("screenshot.png"), &image.png)?;
        }
    }
    let mut full = report.clone();
    for field in &mut full.fields {
        field.included = true;
    }
    full.include_excerpt = true;
    std::fs::write(private.join("diagnostics.md"), full.markdown())?;
    let private_log_saved = private_log.is_some_and(|path| std::fs::copy(path, private.join("session.log")).is_ok());
    if let Some(dir) = session_dir {
        for name in ["panic.txt", "native.txt", "failure.txt"] {
            match std::fs::copy(dir.join(name), private.join(name)) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    std::fs::write(
        private.join("README.txt"),
        "PRIVATE LOCAL DIAGNOSTICS. Never upload this folder to a public issue.\nThe session log and unchecked information may contain private data.\n",
    )?;
    Ok(Saved {
        public_dir: public,
        private_log_saved,
    })
}

pub fn open_directory(path: &Path) -> std::io::Result<()> {
    crate::open_path::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_bundle_uses_saved_log_and_keeps_raw_traces_out_of_public_files() {
        let root = std::env::temp_dir().join(format!("aurora-crash-bundle-test-{}", uuid::Uuid::new_v4()));
        let session = root.join("old-session");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("session.log"), "[old ERROR aurora_viewer] panic: PRIVATE_LOG_NAME\n").unwrap();
        std::fs::write(session.join("panic.txt"), "Rust panic: panicked at PRIVATE_TRACE_PATH\n").unwrap();
        std::fs::write(
            session.join("native.txt"),
            "Native exception 0xC0000005\nFault address: PRIVATE_NATIVE\n",
        )
        .unwrap();
        let diagnostics = preserved_diagnostics(&session);
        assert!(diagnostics.fields.is_empty());
        assert!(diagnostics.excerpt.contains("Rust panic"));
        assert!(diagnostics.excerpt.contains("native exception"));
        assert!(!diagnostics.excerpt.contains("PRIVATE_"));
        assert!(!diagnostics.excerpt.contains("0xC0000005"));
        let report = Report {
            title: "Crash".into(),
            excerpt: diagnostics.excerpt,
            ..Default::default()
        };
        let saved = save_in(&root, &report, None, false, Some(&session.join("session.log")), Some(&session)).unwrap();
        let public = std::fs::read_to_string(saved.public_dir.join("report.md")).unwrap();
        assert!(!public.contains("PRIVATE_"));
        assert!(!public.contains("Rust panic"));
        let private = saved.public_dir.parent().unwrap().join("private-do-not-upload");
        for (name, marker) in [
            ("panic.txt", "PRIVATE_TRACE_PATH"),
            ("native.txt", "PRIVATE_NATIVE"),
            ("session.log", "PRIVATE_LOG_NAME"),
        ] {
            assert!(std::fs::read_to_string(private.join(name)).unwrap().contains(marker));
        }
        assert_eq!(std::fs::read_dir(&saved.public_dir).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_bundle_contains_only_reviewed_fields_and_private_log_is_separate() {
        let root = std::env::temp_dir().join(format!("aurora-report-test-{}", uuid::Uuid::new_v4()));
        let report = Report {
            title: "Example".into(),
            fields: vec![Field::new("test", "Location", "SECRET_LOCATION", false)],
            ..Default::default()
        };
        std::fs::create_dir_all(&root).unwrap();
        let log = root.join("input.log");
        std::fs::write(&log, b"PRIVATE_LOG").unwrap();
        let image = capture(1, 1, vec![255; 4]).unwrap();
        let saved = save_in(&root, &report, Some(&image), false, Some(&log), None).unwrap();
        let public = std::fs::read_to_string(saved.public_dir.join("report.md")).unwrap();
        assert!(!public.contains("SECRET_LOCATION"));
        assert!(!public.contains("PRIVATE_LOG"));
        assert_eq!(std::fs::read_dir(&saved.public_dir).unwrap().count(), 1);
        assert!(saved.private_log_saved);
        assert_eq!(
            std::fs::read(saved.public_dir.parent().unwrap().join("private-do-not-upload/session.log")).unwrap(),
            b"PRIVATE_LOG"
        );
        // Only the freshly generated test directory is removed.
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The conversation transcripts are never part of a report: the bundle
    /// copies a closed list of files, and the transcripts live under the
    /// settings folder, outside what a report reads.
    #[test]
    fn conversation_transcripts_never_enter_a_bundle() {
        use crate::world::chat_log::store::{Identity, folder};
        let root = std::env::temp_dir().join(format!("aurora-report-chatlog-test-{}", uuid::Uuid::new_v4()));
        // transcripts dropped right next to the session log, the worst case
        let session = root.join("session");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("session.log"), "[INFO aurora_viewer] started\n").unwrap();
        for name in ["chat.txt", "Jane Doe.txt", "Les Loups (group).txt"] {
            std::fs::write(session.join(name), "[2026/10/12 14:05]  Jane Doe: PRIVATE_CHAT_LINE\n").unwrap();
        }
        let report = Report {
            title: "Example".into(),
            ..Default::default()
        };
        let saved = save_in(&root, &report, None, false, Some(&session.join("session.log")), Some(&session)).unwrap();
        let bundle = saved.public_dir.parent().unwrap().to_path_buf();
        fn files(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                if e.path().is_dir() {
                    files(&e.path(), out);
                } else {
                    out.push(e.path());
                }
            }
        }
        let mut written = Vec::new();
        files(&bundle, &mut written);
        assert!(!written.is_empty());
        for path in &written {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                ["report.md", "diagnostics.md", "README.txt", "session.log"].contains(&name.as_str()),
                "unexpected file in a report: {name}"
            );
            assert!(!String::from_utf8_lossy(&std::fs::read(path).unwrap()).contains("PRIVATE_CHAT_LINE"));
        }
        // where the transcripts really are: not under the logs, the crash
        // sessions or the saved reports
        let transcripts = folder(
            &crate::settings::accounts_dir(),
            &Identity {
                grid: "agni".into(),
                agent: uuid::Uuid::from_u128(1),
            },
        );
        let logs = crate::logging::logs_dir();
        assert!(!transcripts.starts_with(&logs));
        assert!(!transcripts.starts_with(logs.parent().unwrap().join("bug-reports")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn capture_is_reencoded_with_bounded_preview_and_no_source_metadata() {
        let image = capture(2, 1, vec![255; 8]).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert!(image.png.starts_with(b"\x89PNG"));
        assert_eq!(image.preview.size, [2, 1]);
        assert!(capture(2, 2, vec![0; 2]).is_err());
    }
}
