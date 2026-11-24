//! Windows x86-64: NVIDIA's signed Streamline release (mod.rs), as
//! scripts/dlss-setup.ps1 installs it.
//!
//! The release is one zip of 276 MB, nearly all of it what the viewer does
//! not use (debug builds, other features, samples). Its central directory
//! is read from its end and only the six entries below are fetched, by
//! HTTP range requests (about 32 MB). Each is checked against the SHA-256
//! of the entry in NVIDIA's zip (whose own SHA-256 is the script's), and
//! each DLL against NVIDIA's Authenticode signature. A server that does not
//! answer ranges gets the whole zip, checked against its SHA-256.

use super::{Net, Report, Step};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// NVIDIA's Streamline release, as scripts/dlss-setup.ps1 pins it.
const URL: &str = "https://github.com/NVIDIA-RTX/Streamline/releases/download/v2.14.16/streamline-sdk-v2.14.16.zip";
const ZIP_SIZE: u64 = 276_016_170;
const ZIP_SHA256: &str = "32041e51db0cab709a271b72a352281b690c2abbd669a827809401a5dd23956b";

/// A file in the zip: its entry, the name it is installed under, its size,
/// SHA-256, and its compressed size (for the progress bar).
struct Entry {
    zip: &'static str,
    name: &'static str,
    size: u64,
    sha256: &'static str,
    packed: u64,
}

const LICENCE: Entry = Entry {
    zip: "bin/x64/nvngx_dlss.license.txt",
    name: super::LICENCE_NAME,
    size: 26_727,
    sha256: "3027f23ca5a46dd9cb8183fbd522983a86f64d7daac5982912bf9f214671f294",
    packed: 9_256,
};

/// What goes in the folder, besides NVIDIA's licence the user accepted.
const FILES: [Entry; 5] = [
    Entry {
        zip: "bin/x64/sl.interposer.dll",
        name: "sl.interposer.dll",
        size: 652_928,
        sha256: "b1d7395aebd7b9722841d64dcf7433680020b6a8e421f851c16d5ba0c2182edf",
        packed: 284_521,
    },
    Entry {
        zip: "bin/x64/sl.common.dll",
        name: "sl.common.dll",
        size: 843_904,
        sha256: "38eea3b4209afc455636ea372b5933d65fcc13becec6c84d101dcfc4d8648f75",
        packed: 390_611,
    },
    Entry {
        zip: "bin/x64/sl.dlss.dll",
        name: "sl.dlss.dll",
        size: 422_016,
        sha256: "18871eea87c90345da97d8109faa81ea7d5840e8d99b51b55b0e46cfa177f84e",
        packed: 194_963,
    },
    Entry {
        zip: "bin/x64/nvngx_dlss.dll",
        name: "nvngx_dlss.dll",
        size: 58_956_912,
        sha256: "d00b5b0ab29c3ca101b70c3a5ecb3a01b7ef33766a3383a28a18623083e6123e",
        packed: 31_123_363,
    },
    // Streamline's own licence (MIT)
    Entry {
        zip: "license.txt",
        name: "STREAMLINE-LICENSE.txt",
        size: 1_591,
        sha256: "7b6f23e7d6f3ad6292f9308d2b42cdc3d82ae4e9b2abb55f230279d83bedd43d",
        packed: 883,
    },
];

pub const INSTALLED: [&str; 5] = [FILES[0].name, FILES[1].name, FILES[2].name, FILES[3].name, FILES[4].name];

/// NVIDIA's DLSS licence, from the release.
pub fn licence(net: &Net) -> Result<String, String> {
    let mut zip = open(net, None, &|_| {})?;
    let bytes = read(&mut zip, &LICENCE)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The files of [`INSTALLED`], checked, in `work`.
pub fn prepare(net: &Net, work: &Path, report: Report) -> Result<(), String> {
    let total: u64 = FILES.iter().map(|f| f.packed).sum();
    let fetched = |got: u64| report(Step::Downloading, 0.95 * (got.min(total) as f32 / total as f32));
    let mut zip = open(net, Some(work), &fetched)?;
    for f in &FILES {
        let bytes = read(&mut zip, f)?;
        let path = work.join(f.name);
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", f.name))?;
        if f.name.ends_with(".dll") {
            signed_by_nvidia(&path).map_err(|e| format!("{} is not signed by NVIDIA ({e})", f.name))?;
        }
    }
    Ok(())
}

/// What an earlier version of the setup put there in Streamline's place.
pub fn remove_old(folder: &Path) {
    let _ = std::fs::remove_file(folder.join("nvsdk_ngx.dll"));
}

/// The release's zip: read by ranges where the server allows it, else
/// downloaded whole into `work` (when given) and checked.
fn open<'a>(net: &'a Net, work: Option<&Path>, fetched: &'a dyn Fn(u64)) -> Result<zip::ZipArchive<Source<'a>>, String> {
    let source = match Ranges::new(net, fetched)? {
        Some(r) => Source::Ranges(r),
        None => {
            let dir = work.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
            Source::File(whole(net, &dir, fetched)?)
        }
    };
    zip::ZipArchive::new(source).map_err(|e| format!("NVIDIA's Streamline zip: {e}"))
}

fn read(zip: &mut zip::ZipArchive<Source>, e: &Entry) -> Result<Vec<u8>, String> {
    let mut file = zip.by_name(e.zip).map_err(|err| format!("{} in NVIDIA's zip: {err}", e.zip))?;
    if file.size() != e.size {
        return Err(format!("{} in NVIDIA's zip is not the file expected", e.zip));
    }
    let mut out = Vec::with_capacity(e.size as usize);
    file.read_to_end(&mut out).map_err(|err| format!("{}: {err}", e.zip))?;
    if out.len() as u64 != e.size || super::sha256(&out) != e.sha256 {
        return Err(format!("{} in NVIDIA's zip is not the file expected", e.zip));
    }
    Ok(out)
}

/// The whole zip, for a server that does not answer ranges.
fn whole(net: &Net, dir: &Path, fetched: &dyn Fn(u64)) -> Result<std::fs::File, String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let path = dir.join("streamline.zip");
    let mut file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::new();
    let mut got = 0u64;
    net.rt.block_on(async {
        let mut resp = net
            .client
            .get(URL)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("NVIDIA's Streamline zip: {e}"))?;
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("NVIDIA's Streamline zip: {e}"))? {
            got += chunk.len() as u64;
            if got > ZIP_SIZE {
                return Err(String::from("NVIDIA's Streamline zip is larger than expected"));
            }
            hash.update(&chunk);
            file.write_all(&chunk).map_err(|e| format!("{}: {e}", path.display()))?;
            // (the bar counts the entries' share of the zip)
            fetched(got * 32 / 276);
        }
        Ok::<_, String>(())
    })?;
    let hash: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != ZIP_SIZE || hash != ZIP_SHA256 {
        return Err(String::from("the download is not NVIDIA's Streamline zip"));
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(file)
}

enum Source<'a> {
    Ranges(Ranges<'a>),
    File(std::fs::File),
}

impl Read for Source<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Source::Ranges(r) => r.read(buf),
            Source::File(f) => f.read(buf),
        }
    }
}

impl Seek for Source<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        match self {
            Source::Ranges(r) => r.seek(to),
            Source::File(f) => f.seek(to),
        }
    }
}

/// The zip on the server, read as a file: each read past what is held asks
/// for the next span (small where the reader jumps about, the central
/// directory and the entries' headers; large where it reads on, an
/// entry's data).
struct Ranges<'a> {
    net: &'a Net,
    fetched: &'a dyn Fn(u64),
    pos: u64,
    /// The spans held, the latest last.
    held: Vec<(u64, Vec<u8>)>,
    got: u64,
}

impl<'a> Ranges<'a> {
    const SMALL: u64 = 64 << 10;
    const LARGE: u64 = 4 << 20;

    /// `None` when the server does not answer ranges.
    fn new(net: &'a Net, fetched: &'a dyn Fn(u64)) -> Result<Option<Self>, String> {
        let mut r = Self {
            net,
            fetched,
            pos: 0,
            held: Vec::new(),
            got: 0,
        };
        // the zip's end: its central directory and the record pointing at it
        let start = ZIP_SIZE - Self::SMALL;
        match r.get(start, ZIP_SIZE) {
            Ok(Some(bytes)) => {
                r.held.push((start, bytes));
                Ok(Some(r))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Bytes `start..end`, or `None` when the server sent something else
    /// than that range.
    fn get(&mut self, start: u64, end: u64) -> Result<Option<Vec<u8>>, String> {
        let out = self.net.rt.block_on(async {
            let mut resp = self
                .net
                .client
                .get(URL)
                .header(reqwest::header::RANGE, format!("bytes={start}-{}", end - 1))
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("NVIDIA's Streamline zip: {e}"))?;
            let whole = format!("/{ZIP_SIZE}");
            let ranged = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT
                && resp
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.ends_with(&whole) && v.starts_with(&format!("bytes {start}-")));
            if !ranged {
                return Ok(None);
            }
            let mut out = Vec::with_capacity((end - start) as usize);
            while let Some(chunk) = resp.chunk().await.map_err(|e| format!("NVIDIA's Streamline zip: {e}"))? {
                out.extend_from_slice(&chunk);
                if out.len() as u64 > end - start {
                    return Err(String::from("NVIDIA's Streamline zip: more than the range asked"));
                }
            }
            Ok(Some(out).filter(|o| o.len() as u64 == end - start))
        })?;
        if let Some(o) = &out {
            self.got += o.len() as u64;
            (self.fetched)(self.got);
        }
        Ok(out)
    }
}

impl Read for Ranges<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= ZIP_SIZE || buf.is_empty() {
            return Ok(0);
        }
        let pos = self.pos;
        let found = self
            .held
            .iter()
            .rposition(|(start, bytes)| (*start..*start + bytes.len() as u64).contains(&pos));
        let i = match found {
            Some(i) => i,
            None => {
                // reading on from the last span: an entry's data
                let on = self.held.last().is_some_and(|(s, b)| s + b.len() as u64 == pos);
                let span = if on { Self::LARGE } else { Self::SMALL }.max(buf.len() as u64);
                let end = (pos + span).min(ZIP_SIZE);
                let bytes = self
                    .get(pos, end)
                    .map_err(std::io::Error::other)?
                    .ok_or_else(|| std::io::Error::other("the server stopped answering ranges"))?;
                // (the zip's end stays: the reader goes back to it per entry)
                if self.held.len() > 3 {
                    self.held.remove(1);
                }
                self.held.push((pos, bytes));
                self.held.len() - 1
            }
        };
        let (start, bytes) = &self.held[i];
        let from = (pos - start) as usize;
        let n = buf.len().min(bytes.len() - from);
        buf[..n].copy_from_slice(&bytes[from..from + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Ranges<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => ZIP_SIZE.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| std::io::Error::other("seek before the start"))?;
        Ok(self.pos)
    }
}

/// `path` carries a valid Authenticode signature whose signer's
/// organisation is NVIDIA Corporation (the script's check).
fn signed_by_nvidia(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Cryptography::{CERT_NAME_ATTR_TYPE, CertGetNameStringW};
    use windows_sys::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
        WTHelperProvDataFromStateData, WinVerifyTrust,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: wide.as_ptr(),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    data.Anonymous.pFile = &mut file;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and what it points at (`file`, `wide`) outlive the
    // calls; the state the verify opens is closed below whatever happened.
    unsafe {
        let status = WinVerifyTrust(std::ptr::null_mut(), &mut action, (&raw mut data).cast());
        let organisation = (status == 0)
            .then(|| {
                let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
                let signer = if provider.is_null() {
                    std::ptr::null_mut()
                } else {
                    WTHelperGetProvSignerFromChain(provider, 0, 0, 0)
                };
                if signer.is_null() || (*signer).csCertChain == 0 {
                    return None;
                }
                let cert = (*(*signer).pasCertChain).pCert;
                let oid = c"2.5.4.10"; // szOID_ORGANIZATION_NAME
                let mut name = [0u16; 256];
                let n = CertGetNameStringW(
                    cert,
                    CERT_NAME_ATTR_TYPE,
                    0,
                    oid.as_ptr().cast(),
                    name.as_mut_ptr(),
                    name.len() as u32,
                );
                Some(String::from_utf16_lossy(&name[..(n as usize).saturating_sub(1)]))
            })
            .flatten();
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(std::ptr::null_mut(), &mut action, (&raw mut data).cast());
        match organisation {
            Some(o) if o == "NVIDIA Corporation" => Ok(()),
            Some(o) => Err(format!("signed by {o}")),
            None if status != 0 => Err(format!("signature check 0x{:08x}", status as u32)),
            None => Err(String::from("no signer")),
        }
    }
}
