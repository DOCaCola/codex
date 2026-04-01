use std::path::Path;
use std::path::PathBuf;

use codex_shell_command::parse_command::extract_shell_command;
use dirs::home_dir;
use shlex::try_join;

pub(crate) fn escape_command(command: &[String]) -> String {
    try_join(command.iter().map(String::as_str)).unwrap_or_else(|_| command.join(" "))
}

fn normalize_command_display(command_display: String) -> String {
    let stripped = strip_msys_export_prefix_for_display(&command_display);
    if stripped.is_empty() || stripped == command_display {
        command_display
    } else {
        stripped.to_string()
    }
}

fn strip_msys_export_prefix_for_display(command_display: &str) -> &str {
    let trimmed = command_display.trim_start();
    let Some(rest) = trimmed.strip_prefix("export ") else {
        return command_display;
    };
    let Some((exports, remainder)) = rest.split_once(';') else {
        return command_display;
    };

    let mut saw_supported_export = false;
    for assignment in exports.split_whitespace() {
        if assignment.starts_with("MSYSTEM=") || assignment.starts_with("CHERE_INVOKING=") {
            saw_supported_export = true;
        } else {
            return command_display;
        }
    }

    if !saw_supported_export {
        return command_display;
    }

    remainder.trim_start()
}

pub(crate) fn strip_bash_lc_and_escape(command: &[String]) -> String {
    if let Some((_, script)) = extract_shell_command(command) {
        return normalize_command_display(script.to_string());
    }
    normalize_command_display(escape_command(command))
}

pub(crate) fn split_command_string(command: &str) -> Vec<String> {
    let Some(parts) = shlex::split(command) else {
        return vec![command.to_string()];
    };
    match shlex::try_join(parts.iter().map(String::as_str)) {
        Ok(round_trip)
            if round_trip == command
                || (!command.contains(":\\")
                    && shlex::split(&round_trip).as_ref() == Some(&parts)) =>
        {
            parts
        }
        _ => vec![command.to_string()],
    }
}

/// If `path` is absolute and inside $HOME, return the part *after* the home
/// directory; otherwise, return the path as-is. Note if `path` is the homedir,
/// this will return and empty path.
pub(crate) fn relativize_to_home<P>(path: P) -> Option<PathBuf>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    if !path.is_absolute() {
        // If the path is not absolute, we can’t do anything with it.
        return None;
    }

    let home_dir = home_dir()?;
    let rel = path.strip_prefix(&home_dir).ok()?;
    Some(rel.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_command() {
        let args = vec!["foo".into(), "bar baz".into(), "weird&stuff".into()];
        let cmdline = escape_command(&args);
        assert_eq!(cmdline, "foo 'bar baz' 'weird&stuff'");
    }

    #[test]
    fn test_strip_bash_lc_and_escape() {
        // Test bash
        let args = vec!["bash".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");

        // Test zsh
        let args = vec!["zsh".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");

        // Test absolute path to zsh
        let args = vec!["/usr/bin/zsh".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");

        // Test absolute path to bash
        let args = vec!["/bin/bash".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");
    }

    #[test]
    fn test_strip_bash_lc_and_escape_hides_msys_export_prefix() {
        let args = vec![
            "bash".into(),
            "-lc".into(),
            "export MSYSTEM=UCRT64 CHERE_INVOKING=1; sed -n '1,220p' /c/tmp/log.txt".into(),
        ];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "sed -n '1,220p' /c/tmp/log.txt");
    }

    #[test]
    fn test_strip_bash_lc_and_escape_keeps_other_exports() {
        let args = vec![
            "bash".into(),
            "-lc".into(),
            "export FOO=bar; sed -n '1,220p' /c/tmp/log.txt".into(),
        ];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "export FOO=bar; sed -n '1,220p' /c/tmp/log.txt");
    }

    #[test]
    fn split_command_string_round_trips_shell_wrappers() {
        let command =
            shlex::try_join(["/bin/zsh", "-lc", r#"python3 -c 'print("Hello, world!")'"#])
                .expect("round-trippable command");
        assert_eq!(
            split_command_string(&command),
            vec![
                "/bin/zsh".to_string(),
                "-lc".to_string(),
                r#"python3 -c 'print("Hello, world!")'"#.to_string(),
            ]
        );
    }

    #[test]
    fn split_command_string_preserves_non_roundtrippable_windows_commands() {
        let command = r#"C:\Program Files\Git\bin\bash.exe -lc "echo hi""#;
        assert_eq!(split_command_string(command), vec![command.to_string()]);
    }
}
