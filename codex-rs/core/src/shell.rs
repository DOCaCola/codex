use codex_exec_server::ShellInfo;
use codex_shell_command::shell_detect::DetectedShell;
use crate::shell_snapshot::ShellSnapshot;
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;

pub use codex_shell_command::shell_detect::ShellType;

#[derive(Clone, Serialize, Deserialize)]
pub struct Shell {
    pub(crate) shell_type: ShellType,
    pub(crate) shell_path: PathBuf,
    #[serde(
        skip_serializing,
        skip_deserializing,
        default = "empty_shell_snapshot_receiver"
    )]
    pub(crate) shell_snapshot: watch::Receiver<Option<Arc<ShellSnapshot>>>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("shell_type", &self.shell_type)
            .field("shell_path", &self.shell_path)
            .finish()
    }
}

impl Shell {
    pub fn name(&self) -> &'static str {
        self.shell_type.name()
    }

    /// Takes a string of shell and returns the full list of command args to
    /// use with `exec()` to run the shell command.
    pub fn derive_exec_args(&self, command: &str, use_login_shell: bool) -> Vec<String> {
        match self.shell_type {
            ShellType::Zsh | ShellType::Bash | ShellType::Sh => {
                let arg = if use_login_shell { "-lc" } else { "-c" };
                vec![
                    self.shell_path.to_string_lossy().to_string(),
                    arg.to_string(),
                    command.to_string(),
                ]
            }
            ShellType::PowerShell => {
                let mut args = vec![self.shell_path.to_string_lossy().to_string()];
                if !use_login_shell {
                    args.push("-NoProfile".to_string());
                }

                args.push("-Command".to_string());
                args.push(command.to_string());
                args
            }
            ShellType::Cmd => {
                let mut args = vec![self.shell_path.to_string_lossy().to_string()];
                args.push("/c".to_string());
                args.push(command.to_string());
                args
            }
        }
    }
}

impl From<DetectedShell> for Shell {
    fn from(detected: DetectedShell) -> Self {
        Self {
            shell_type: detected.shell_type,
            shell_path: detected.shell_path,
            shell_snapshot: empty_shell_snapshot_receiver(),
        }
    }
}

pub(crate) fn empty_shell_snapshot_receiver() -> watch::Receiver<Option<Arc<ShellSnapshot>>> {
    let (_tx, rx) = watch::channel(None);
    rx
}

impl Shell {
    pub(crate) fn from_environment_shell_info(shell_info: ShellInfo) -> anyhow::Result<Self> {
        let shell_type = match shell_info.name.as_str() {
            "zsh" => ShellType::Zsh,
            "bash" => ShellType::Bash,
            "powershell" => ShellType::PowerShell,
            "sh" => ShellType::Sh,
            "cmd" => ShellType::Cmd,
            name => anyhow::bail!("unknown environment shell `{name}`"),
        };

        Ok(Self {
            shell_type,
            shell_path: PathBuf::from(shell_info.path),
            shell_snapshot: empty_shell_snapshot_receiver(),
        })
    }
}

#[cfg(all(test, unix))]
fn ultimate_fallback_shell() -> Shell {
    codex_shell_command::shell_detect::ultimate_fallback_shell().into()
}

pub fn get_shell_by_model_provided_path(shell_path: &PathBuf) -> Shell {
    codex_shell_command::shell_detect::get_shell_by_model_provided_path(shell_path).into()
}

pub fn get_shell(shell_type: ShellType, path: Option<&PathBuf>) -> Option<Shell> {
    codex_shell_command::shell_detect::get_shell(shell_type, path).map(Into::into)
}

pub fn default_user_shell() -> Shell {
    codex_shell_command::shell_detect::default_user_shell().into()
}

#[cfg(all(test, target_os = "macos"))]
fn default_user_shell_from_path(user_shell_path: Option<PathBuf>) -> Shell {
    codex_shell_command::shell_detect::default_user_shell_from_path(user_shell_path).into()
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
