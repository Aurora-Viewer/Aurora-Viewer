mod build_support;
mod git_watch;

use std::path::Path;

use git_watch::git;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support.rs");
    println!("cargo:rerun-if-changed=git_watch.rs");
    println!("cargo:rerun-if-env-changed=AURORA_VERSION_TAG");
    println!("cargo:rerun-if-env-changed=AURORA_UPDATE_TEST_VERSION");
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies CARGO_MANIFEST_DIR");
    let root = Path::new(&manifest).join("../..");

    // Cargo must rebuild after a commit, a checkout or a tag of this
    // checkout, and not after the Git commands of another worktree.
    for path in git_watch::watch_list(&root) {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let tag = std::env::var("AURORA_VERSION_TAG").ok();
    let description = git(
        &root,
        &["describe", "--tags", "--match", "v[0-9]*.[0-9]*.[0-9]*", "--long", "--abbrev=9"],
    );
    let commit = git(&root, &["rev-parse", "--short=9", "HEAD"]);
    let count = git(&root, &["rev-list", "--count", "HEAD"])
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    let fixture = std::env::var("AURORA_UPDATE_TEST_VERSION").ok();
    if fixture.is_some() && (std::env::var("PROFILE").as_deref() != Ok("debug") || tag.is_some()) {
        panic!("Update fixtures require a dev build without AURORA_VERSION_TAG");
    }
    // Two disposable installations exercise real version-to-version replacement
    // without creating Git tags or publishing releases. Such binaries refuse
    // normal startup; aurora-updater requires an isolated loopback fixture.
    let version = build_support::resolve(
        fixture.as_deref().or(tag.as_deref()),
        if fixture.is_some() { None } else { description.as_deref() },
        count,
        commit.as_deref(),
    )
    .unwrap_or_else(|error| panic!("Cannot determine Aurora's version: {error}"));
    println!(
        "cargo:rustc-env=AURORA_UPDATE_TEST_BUILD={}",
        if fixture.is_some() { "1" } else { "0" }
    );
    println!("cargo:rustc-env=AURORA_COMPILED_VERSION={}", version.display);
    println!("cargo:rustc-env=AURORA_LOGIN_VERSION={}", version.login);
    println!("cargo:rustc-env=AURORA_BUILD_COMMIT={}", commit.as_deref().unwrap_or("unknown"));
    // `links` metadata, read by aurora-viewer's build script as
    // DEP_AURORA_VERSION_DISPLAY / _NUMERIC (version block of the exe).
    println!("cargo:display={}", version.display);
    println!("cargo:numeric={}", version.login);
}
