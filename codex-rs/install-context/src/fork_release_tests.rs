use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parses_fork_release_tags_and_orders_numeric_revisions() {
    for version in ["0.166.0-doca", "0.166.0-doca.1", "0.167.0-alpha.1-doca"] {
        assert_eq!(
            version_from_release_tag(&format!("doca-v{version}")),
            Some(Version::parse(version).unwrap())
        );
    }
    for (latest, current, expected) in [
        ("0.166.0-doca.10", "0.166.0-doca.9", Some(true)),
        ("0.166.0-doca.1", "0.166.0-doca", Some(true)),
        ("0.167.0-doca", "0.166.0-doca.10", Some(true)),
        ("0.166.0-doca.1", "0.166.0-doca.1", Some(false)),
        ("0.166.0-doca", "0.167.0-doca", Some(false)),
        ("0.167.0-doca", "0.167.0-alpha.1-doca", Some(true)),
        ("0.167.0", "0.166.0-doca", None),
        ("0.167.0-alpha.1-doca", "0.166.0-doca", None),
    ] {
        assert_eq!(is_newer_fork_release(latest, current), expected);
    }
}

#[test]
fn stable_channel_excludes_alpha_local_and_stock_versions() {
    for version in ["0.166.0-doca", "0.166.0-doca.12"] {
        assert!(is_stable_fork_version(version));
    }
    for version in [
        "0.166.0",
        "0.0.0-doca",
        "0.166.0-alpha.1-doca",
        "0.166.0-doca.1+local",
        "0.166.0-doca.01",
        "0.166.0-doca.invalid",
        "0.166.0-doca.1.2",
    ] {
        assert!(!is_stable_fork_version(version), "{version}");
    }
}
