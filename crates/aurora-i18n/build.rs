use std::path::Path;

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies the manifest directory");
    let locales = Path::new(&manifest).join("locales");
    println!("cargo:rerun-if-changed={}", locales.display());
    let mut paths: Vec<_> = std::fs::read_dir(&locales)
        .expect("bundled locales directory")
        .map(|entry| entry.expect("bundled locale entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "ftl"))
        .collect();
    paths.sort();
    let mut source = String::from("pub const SOURCES: &[(&str, &str)] = &[\n");
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let language = path.file_stem().and_then(|name| name.to_str()).expect("UTF-8 locale name");
        source.push_str(&format!("({language:?}, include_str!({:?})),\n", path.to_string_lossy()));
    }
    source.push_str("];\n");
    let output = std::env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR");
    std::fs::write(Path::new(&output).join("locales.rs"), source).expect("write bundled locale index");
}
