//! Release identity shared by the fork CLI and its managed background server.

use semver::Version;

pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/DOCaCola/codex/releases/latest";
pub const UNIX_INSTALLER_URL: &str =
    "https://github.com/DOCaCola/codex/releases/latest/download/install.sh";
pub const WINDOWS_INSTALLER_URL: &str =
    "https://github.com/DOCaCola/codex/releases/latest/download/install.ps1";
pub const VERSION_CACHE_FILENAME: &str = "doca-version.json";

/// Parse a tag produced by the fork release workflow, including explicit alpha releases.
pub fn version_from_release_tag(tag: &str) -> Option<Version> {
    let raw = tag.strip_prefix("doca-v")?;
    let (base, revision) = raw.rsplit_once("-doca")?;
    Version::parse(base).ok()?;
    if !revision.is_empty()
        && revision
            .strip_prefix('.')
            .and_then(|n| n.parse::<u64>().ok())
            .is_none()
    {
        return None;
    }
    let version = Version::parse(raw).ok()?;
    version.build.is_empty().then_some(version)
}

/// Fork stable releases use `-doca` or `-doca.N`; upstream alpha builds stay pinned.
pub fn is_stable_fork_version(raw: &str) -> bool {
    let Some(version) = version_from_release_tag(&format!("doca-v{raw}")) else {
        return false;
    };
    (version.major, version.minor, version.patch) != (0, 0, 0)
        && (version.pre.as_str() == "doca" || version.pre.as_str().starts_with("doca."))
}

/// Compare published fork releases without accepting a stock version cached by another CLI.
pub fn is_newer_fork_release(latest: &str, current: &str) -> Option<bool> {
    if !is_stable_fork_version(latest) {
        return None;
    }
    Some(Version::parse(latest).ok()? > Version::parse(current.trim()).ok()?)
}

#[cfg(test)]
#[path = "fork_release_tests.rs"]
mod tests;
