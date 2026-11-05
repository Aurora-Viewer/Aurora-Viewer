//! The release key and the signed update manifest (docs/UPDATES.md, "Update
//! signature"). Run with `cargo run -p aurora-updater --bin
//! aurora-update-sign -- <command>`:
//!
//! - `keygen --out <file>`: a new release key. The private key goes to a new
//!   file and is never shown; the public key is printed as the line to add
//!   to `UPDATE_KEYS`.
//! - `sign --tag <vX.Y.Z> --out <manifest> <file>…`: writes the manifest of
//!   those files and its `.sig`, with the private key of `UPDATE_SIGNING_KEY`
//!   (the Release workflow) or of `--key-file`. A key that is not in
//!   `UPDATE_KEYS` is refused: viewers would refuse the release.
//!   `--ephemeral-key` signs with a throwaway key kept in memory and prints
//!   its public key (scripts/test-update.ps1).
//! - `verify --manifest <manifest> [--tag <vX.Y.Z>] [--key <hex>] <file>…`:
//!   checks a published manifest, its signature and the files it names.

use anyhow::{Context, Result, bail, ensure};
use aurora_updater::signature::{self, Entry, Manifest};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aurora-update-sign: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    options: HashMap<&'static str, String>,
    ephemeral: bool,
    files: Vec<PathBuf>,
}

const OPTIONS: [&str; 5] = ["--out", "--tag", "--key-file", "--manifest", "--key"];

fn parse(args: &[String]) -> Result<Args> {
    let mut parsed = Args::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(option) = OPTIONS.iter().find(|o| **o == arg) {
            let value = args.next().with_context(|| format!("{option} needs a value"))?;
            parsed.options.insert(option, value.clone());
        } else if arg == "--ephemeral-key" {
            parsed.ephemeral = true;
        } else if arg.starts_with("--") {
            bail!("unknown option {arg}");
        } else {
            parsed.files.push(arg.into());
        }
    }
    Ok(parsed)
}

impl Args {
    fn get(&self, option: &str) -> Result<&str> {
        self.options
            .get(option)
            .map(String::as_str)
            .with_context(|| format!("{option} is required"))
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let Some((command, rest)) = args.split_first() else {
        bail!("usage: aurora-update-sign keygen | sign | verify (see the comment at the top of this tool)");
    };
    let args = parse(rest)?;
    match command.as_str() {
        "keygen" => keygen(Path::new(args.get("--out")?)),
        "sign" => sign(&args).map(|public| {
            if let Some(public) = public {
                println!("{}", signature::hex(&public));
            }
        }),
        "verify" => verify(&args),
        other => bail!("unknown command {other}"),
    }
}

fn keygen(out: &Path) -> Result<()> {
    let pkcs8 = signature::generate()?;
    let public = signature::public_key(&pkcs8)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(out)
        .with_context(|| format!("cannot create {} (an existing file is never replaced)", out.display()))?;
    file.write_all(format!("{}\n", signature::hex(&pkcs8)).as_bytes())?;
    file.sync_all()?;
    eprintln!("Private key written to {} (not shown). Keep that file offline.", out.display());
    eprintln!("Add the line below to UPDATE_KEYS in crates/aurora-updater/src/signature.rs:");
    println!("    key(\"{}\"),", signature::hex(&public));
    Ok(())
}

/// The manifest's entry for each file, named after the file.
fn entries(files: &[PathBuf]) -> Result<Vec<(Entry, Vec<u8>)>> {
    ensure!(!files.is_empty(), "no file to sign");
    files
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("file name of {}", path.display()))?;
            let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
            Ok((Entry::of(name, &bytes), bytes))
        })
        .collect()
}

fn version(tag: &str) -> Result<String> {
    let version = aurora_updater::feed::version(tag).with_context(|| format!("{tag} is not a SemVer tag"))?;
    ensure!(version.build.is_empty(), "{tag} has build metadata");
    Ok(version.to_string())
}

fn signature_path(manifest: &Path) -> PathBuf {
    let mut path = manifest.as_os_str().to_owned();
    path.push(".sig");
    path.into()
}

/// The private key, never echoed: from `--key-file`, else from the variable.
fn private_key(args: &Args) -> Result<Vec<u8>> {
    let text = match args.options.get("--key-file") {
        Some(path) => fs::read_to_string(path).with_context(|| format!("cannot read the key file {path}"))?,
        None => std::env::var(signature::KEY_VARIABLE)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .with_context(|| format!("no signing key: set {} or pass --key-file", signature::KEY_VARIABLE))?,
    };
    signature::unhex(text.trim()).context("the signing key is not the hex written by keygen")
}

/// Returns the public key of an ephemeral signature.
fn sign(args: &Args) -> Result<Option<[u8; 32]>> {
    let version = version(args.get("--tag")?)?;
    let out = PathBuf::from(args.get("--out")?);
    let entries = entries(&args.files)?;
    for (i, (entry, _)) in entries.iter().enumerate() {
        ensure!(
            entries[..i].iter().all(|(e, _)| e.name != entry.name),
            "{} is given twice",
            entry.name
        );
    }
    let pkcs8 = if args.ephemeral {
        signature::generate()?
    } else {
        private_key(args)?
    };
    let public = signature::public_key(&pkcs8)?;
    if !args.ephemeral && !signature::UPDATE_KEYS.contains(&public) {
        bail!(
            "this key is not in UPDATE_KEYS (crates/aurora-updater/src/signature.rs): viewers would refuse the release. Its public key: {}",
            signature::hex(&public)
        );
    }
    let manifest = Manifest::new(&version, entries.into_iter().map(|(e, _)| e).collect());
    let bytes = manifest.to_bytes();
    let sig = signature::sign(&bytes, &pkcs8)?;
    signature::verify(&bytes, sig.as_bytes(), &[public]).context("the new signature does not verify")?;
    fs::write(&out, &bytes).with_context(|| format!("cannot write {}", out.display()))?;
    fs::write(signature_path(&out), sig)?;
    eprintln!(
        "Signed {} for {version}: {}",
        out.display(),
        manifest.assets.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ")
    );
    Ok(args.ephemeral.then_some(public))
}

fn verify(args: &Args) -> Result<()> {
    let path = PathBuf::from(args.get("--manifest")?);
    let bytes = fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let sig = fs::read(signature_path(&path)).context("cannot read the .sig beside the manifest")?;
    let mut keys = signature::UPDATE_KEYS.to_vec();
    if let Some(key) = args.options.get("--key") {
        keys.push(signature::unhex(key)?.try_into().ok().context("--key: 64 hex digits")?);
    }
    let manifest = signature::verify(&bytes, &sig, &keys)?;
    if let Some(tag) = args.options.get("--tag") {
        ensure!(manifest.version == version(tag)?, "the manifest is for {}", manifest.version);
    }
    for (entry, bytes) in entries(&args.files)? {
        let listed = manifest
            .entry(&entry.name)
            .with_context(|| format!("{} is not in the manifest", entry.name))?;
        ensure!(listed.matches(&bytes), "{} does not match the manifest", entry.name);
    }
    eprintln!(
        "{} verifies: version {}, {} files listed",
        path.display(),
        manifest.version,
        manifest.assets.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp() -> Temp {
        let path = std::env::temp_dir().join(format!("aurora-update-sign-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).expect("temp");
        Temp(path)
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_ephemeral_signature_verifies_and_names_its_files() {
        let dir = temp();
        let (zip, policy, manifest) = (dir.0.join("a.zip"), dir.0.join("p.json"), dir.0.join(signature::MANIFEST));
        fs::write(&zip, b"package").expect("zip");
        fs::write(&policy, b"{}").expect("policy");
        let parsed = parse(&args(&[
            "--tag",
            "v0.3.0",
            "--out",
            manifest.to_str().expect("path"),
            "--ephemeral-key",
            zip.to_str().expect("path"),
            policy.to_str().expect("path"),
        ]))
        .expect("args");
        let public = sign(&parsed).expect("signed").expect("ephemeral public key");
        let key = signature::hex(&public);
        let check = |tag: &str| {
            run(args(&[
                "verify",
                "--manifest",
                manifest.to_str().expect("path"),
                "--tag",
                tag,
                "--key",
                &key,
                zip.to_str().expect("path"),
            ]))
        };
        check("v0.3.0").expect("verifies");
        assert!(check("v0.3.1").is_err(), "another version");
        fs::write(&zip, b"packagf").expect("altered zip");
        assert!(check("v0.3.0").is_err(), "an altered file");
    }

    #[test]
    fn keygen_never_replaces_a_file_and_an_unlisted_key_cannot_sign() {
        let dir = temp();
        let key = dir.0.join("key");
        keygen(&key).expect("keygen");
        assert!(keygen(&key).is_err(), "an existing key file stays");
        let zip = dir.0.join("a.zip");
        fs::write(&zip, b"package").expect("zip");
        let out = dir.0.join(signature::MANIFEST);
        let parsed = parse(&args(&[
            "--tag",
            "v0.3.0",
            "--out",
            out.to_str().expect("path"),
            "--key-file",
            key.to_str().expect("path"),
            zip.to_str().expect("path"),
        ]))
        .expect("args");
        let error = sign(&parsed).expect_err("a key outside UPDATE_KEYS");
        assert!(format!("{error:#}").contains("UPDATE_KEYS"), "{error:#}");
        assert!(!out.exists(), "nothing written");
        assert!(run(args(&["sign", "--tag", "v0.3.0"])).is_err());
        assert!(run(args(&["sign", "--bogus"])).is_err());
    }
}
