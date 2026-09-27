//! Executor-local paths and trusted startup settings used when preparing execution.

use std::path::PathBuf;

use codex_sandboxing::LinuxSandboxPidNamespace;
use codex_utils_absolute_path::AbsolutePathBuf;

pub const CODEX_HPATCH_COMPANION_ARGV0: &str = "__codex_hpatch_companion__";

/// Paths and sandbox settings initialized when creating an executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecServerRuntimeOptions {
    /// Stable path to the Codex executable used to launch hidden helper modes.
    pub codex_self_exe: AbsolutePathBuf,
    /// Path to the Linux sandbox helper alias used when the platform sandbox
    /// needs to re-enter Codex by argv0.
    pub codex_linux_sandbox_exe: Option<AbsolutePathBuf>,
    /// Trusted startup policy; requests and repository config cannot change PID isolation.
    pub linux_sandbox_pid_namespace: LinuxSandboxPidNamespace,
    /// Trusted startup routing; request policy still controls destination access.
    pub proxy_private_ips_via_upstream: bool,
    /// User-config opt-out of writable-root symlink checks beneath this host's home.
    #[cfg(target_os = "macos")]
    pub allowed_symlinked_codex_home: Option<AbsolutePathBuf>,
}

impl ExecServerRuntimeOptions {
    pub fn hpatch_exe(&self) -> std::io::Result<AbsolutePathBuf> {
        let parent = self.codex_self_exe.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Codex executable path has no parent directory",
            )
        })?;
        let file_name = if cfg!(windows) {
            "hpatch.exe"
        } else {
            "hpatch"
        };
        AbsolutePathBuf::from_absolute_path(parent.join(file_name))
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
    }

    pub fn from_optional_paths(
        codex_self_exe: Option<PathBuf>,
        codex_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let codex_self_exe = codex_self_exe.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Codex executable path is not configured",
            )
        })?;
        Self::new(codex_self_exe, codex_linux_sandbox_exe)
    }

    pub fn new(
        codex_self_exe: PathBuf,
        codex_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            linux_sandbox_pid_namespace: LinuxSandboxPidNamespace::default(),
            proxy_private_ips_via_upstream: false,
            codex_self_exe: absolute_path(codex_self_exe)?,
            codex_linux_sandbox_exe: codex_linux_sandbox_exe.map(absolute_path).transpose()?,
            #[cfg(target_os = "macos")]
            allowed_symlinked_codex_home: None,
        })
    }

    /// Applies the PID namespace policy chosen by trusted executor provisioning.
    pub fn with_linux_sandbox_pid_namespace(mut self, mode: LinuxSandboxPidNamespace) -> Self {
        self.linux_sandbox_pid_namespace = mode;
        self
    }

    /// Routes permitted private IPs through the executor's configured upstream proxy.
    pub fn with_proxy_private_ips_via_upstream(mut self, enabled: bool) -> Self {
        self.proxy_private_ips_via_upstream = enabled;
        self
    }

    /// Applies the symlink opt-in resolved by the execution host's config loader.
    #[cfg(target_os = "macos")]
    pub fn with_allowed_symlinked_codex_home(
        mut self,
        allowed_symlinked_codex_home: Option<AbsolutePathBuf>,
    ) -> Self {
        self.allowed_symlinked_codex_home = allowed_symlinked_codex_home;
        self
    }
}

fn absolute_path(path: PathBuf) -> std::io::Result<AbsolutePathBuf> {
    AbsolutePathBuf::from_absolute_path(path.as_path())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hpatch_is_resolved_beside_codex() {
        let codex_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let expected_name = if cfg!(windows) {
            "hpatch.exe"
        } else {
            "hpatch"
        };
        let codex = std::env::temp_dir()
            .join("codex-runtime-path-test")
            .join(codex_name);
        let paths = ExecServerRuntimeOptions::new(codex, None).expect("runtime paths");
        assert_eq!(
            paths
                .hpatch_exe()
                .expect("hpatch path")
                .file_name()
                .and_then(|name| name.to_str()),
            Some(expected_name)
        );
    }
}
