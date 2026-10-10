#[path = "../build_support.rs"]
mod build_support;

use build_support::{BuildVersion, resolve};

#[test]
fn release_uses_the_tag_instead_of_the_package_version() {
    assert_eq!(
        resolve(Some("v0.2.0"), Some("v0.2.0-0-g123456789"), 99, None),
        Ok(BuildVersion {
            display: "0.2.0".into(),
            login: "0.2.0.0".into()
        })
    );
    assert_eq!(resolve(None, Some("v0.2.0-0-g123456789"), 99, None).unwrap().display, "0.2.0");
}

#[test]
fn development_build_includes_distance_and_commit() {
    let version = resolve(None, Some("v0.2.0-7-ge30253a19"), 99, None).unwrap();
    assert_eq!(version.display, "0.2.0+7.ge30253a19");
    assert_eq!(version.login, "0.2.0.7");
    assert!(semver::Version::parse(&version.display).is_ok());
}

#[test]
fn prereleases_and_existing_metadata_are_preserved() {
    assert_eq!(resolve(Some("v1.2.3-beta.1"), None, 0, None).unwrap().display, "1.2.3-beta.1");
    assert_eq!(
        resolve(None, Some("v1.2.3-beta.1+build-2-gabcdef123"), 0, None).unwrap().display,
        "1.2.3-beta.1+build.2.gabcdef123"
    );
}

#[test]
fn release_rejects_a_mismatched_tag_or_a_commit_after_the_tag() {
    assert!(resolve(Some("v0.3.0"), Some("v0.2.0-0-g123456789"), 0, None).is_err());
    assert!(resolve(Some("v0.2.0"), Some("v0.2.0-1-g123456789"), 0, None).is_err());
    for tag in ["0.2.0", "v01.2.0", "v1.2", "v1.2.3\n", "v1.2.3;echo"] {
        assert!(resolve(Some(tag), None, 0, None).is_err(), "{tag}");
    }
    for description in ["v1.2.3", "v1.2.3-x-gabc", "v1.2.3-1-gxxx", "v1.2.3-1-g"] {
        assert!(resolve(None, Some(description), 0, None).is_err(), "{description}");
    }
}

#[test]
fn sources_without_tags_or_git_are_clearly_identified() {
    assert_eq!(resolve(None, None, 12, Some("abcdef123")).unwrap().display, "0.0.0+12.gabcdef123");
    assert_eq!(resolve(None, None, 0, None).unwrap().display, "0.0.0+0.gunknown");
    assert_eq!(resolve(Some("v0.2.0"), None, 0, None).unwrap().display, "0.2.0");
}

#[test]
fn runtime_and_http_read_the_compiled_version() {
    assert_eq!(aurora_version::user_agent(), format!("AuroraViewer/{}", aurora_version::version()));
    let version = semver::Version::parse(aurora_version::version()).unwrap();
    assert!(aurora_version::login_version().starts_with(&format!("{}.{}.{}.", version.major, version.minor, version.patch)));
}
