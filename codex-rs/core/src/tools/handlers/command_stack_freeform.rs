use serde_json::Map;
use serde_json::Value;

use crate::function_tool::FunctionCallError;
use crate::tools::handlers::command_stack::CommandItem;
use crate::tools::handlers::command_stack::CommandStackArgs;
use crate::tools::handlers::command_stack::CommandType;

pub(super) fn parse_command_stack(input: &str) -> Result<CommandStackArgs, FunctionCallError> {
    let lines = input.split('\n').collect::<Vec<_>>();
    let mut commands = Vec::new();
    let mut line_index = 0;

    while line_index < lines.len() {
        let line = strip_carriage_return(lines[line_index]);
        if line.trim().is_empty() {
            line_index += 1;
            continue;
        }

        let header = parse_header(line, line_index + 1)?;
        line_index += 1;
        let body_start_line = line_index + 1;
        let mut body_lines = Vec::new();
        let mut hpatch_heredoc = false;
        let mut terminated = false;

        while line_index < lines.len() {
            let body_line = strip_carriage_return(lines[line_index]);
            if body_line == "end" && !hpatch_heredoc {
                terminated = true;
                line_index += 1;
                break;
            }

            if header.command_type == CommandType::Hpatch {
                if hpatch_heredoc {
                    if body_line == "PATCH" {
                        hpatch_heredoc = false;
                    }
                } else if is_hpatch_heredoc_start(body_line) {
                    hpatch_heredoc = true;
                }
            }

            body_lines.push(body_line);
            line_index += 1;
        }

        if !terminated {
            return Err(model_error(format!(
                "command_stack operation on line {} is missing its closing `end`",
                header.line
            )));
        }
        if hpatch_heredoc {
            return Err(model_error(format!(
                "hpatch body starting on line {body_start_line} has an unterminated heredoc"
            )));
        }

        let body = body_lines.join("\n");
        if body.trim().is_empty() {
            return Err(model_error(format!(
                "command_stack operation on line {} has an empty body",
                header.line
            )));
        }
        let arguments = match header.command_type {
            CommandType::ExecCommand | CommandType::WriteStdin => {
                let arguments: Value = serde_json::from_str(&body).map_err(|err| {
                    model_error(format!(
                        "invalid JSON body for {} operation on line {}: {err}",
                        header.command_type.name(),
                        header.line
                    ))
                })?;
                if !arguments.is_object() {
                    return Err(model_error(format!(
                        "{} body on line {} must be a JSON object",
                        header.command_type.name(),
                        header.line
                    )));
                }
                arguments
            }
            CommandType::ApplyPatch => patch_arguments("patch", body),
            CommandType::Hpatch => patch_arguments("script", body),
        };

        commands.push(CommandItem {
            command_type: header.command_type,
            step: header.step,
            arguments,
            continue_on_failure: header.continue_on_failure,
        });
    }

    Ok(CommandStackArgs { commands })
}

struct OperationHeader {
    command_type: CommandType,
    step: u8,
    continue_on_failure: bool,
    line: usize,
}

fn parse_header(line: &str, line_number: usize) -> Result<OperationHeader, FunctionCallError> {
    let mut tokens = line.split_ascii_whitespace();
    let command_type = match tokens.next() {
        Some("exec_command") => CommandType::ExecCommand,
        Some("write_stdin") => CommandType::WriteStdin,
        Some("apply_patch") => CommandType::ApplyPatch,
        Some("hpatch") => CommandType::Hpatch,
        Some(operation) => {
            return Err(model_error(format!(
                "unknown command_stack operation `{operation}` on line {line_number}"
            )));
        }
        None => unreachable!("blank lines are skipped before parsing headers"),
    };
    let mut step = 1;
    let mut saw_step = false;
    let mut continue_on_failure = false;

    for token in tokens {
        if let Some(value) = token.strip_prefix("step=") {
            if saw_step {
                return Err(model_error(format!(
                    "duplicate step on command_stack line {line_number}"
                )));
            }
            step = value.parse::<u8>().map_err(|_| {
                model_error(format!(
                    "invalid command_stack step `{value}` on line {line_number}"
                ))
            })?;
            saw_step = true;
        } else if token == "continue_on_failure" {
            if continue_on_failure {
                return Err(model_error(format!(
                    "duplicate continue_on_failure on command_stack line {line_number}"
                )));
            }
            continue_on_failure = true;
        } else {
            return Err(model_error(format!(
                "unknown command_stack header option `{token}` on line {line_number}"
            )));
        }
    }

    Ok(OperationHeader {
        command_type,
        step,
        continue_on_failure,
        line: line_number,
    })
}

fn patch_arguments(key: &str, input: String) -> Value {
    let mut arguments = Map::new();
    arguments.insert(key.to_string(), Value::String(input));
    Value::Object(arguments)
}

fn strip_carriage_return(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn is_hpatch_heredoc_start(line: &str) -> bool {
    matches!(line, "type <<PATCH" | "type <<'PATCH'" | "type <<\"PATCH\"")
}

fn model_error(message: String) -> FunctionCallError {
    FunctionCallError::RespondToModel(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uniform_blocks_and_defaults_step_to_one() {
        let parsed = parse_command_stack(
            r#"exec_command
{"cmd":"cargo test","workdir":"codex"}
end
write_stdin step=2 continue_on_failure
{"session_id":42,"chars":""}
end
apply_patch step=3
*** Begin Patch
*** Add File: sample.txt
+hello
*** End Patch
end"#,
        )
        .expect("parse command stack");

        assert_eq!(parsed.commands.len(), 3);
        assert_eq!(parsed.commands[0].command_type, CommandType::ExecCommand);
        assert_eq!(parsed.commands[0].step, 1);
        assert_eq!(parsed.commands[0].arguments["cmd"], "cargo test");
        assert_eq!(parsed.commands[1].step, 2);
        assert!(parsed.commands[1].continue_on_failure);
        assert_eq!(parsed.commands[2].command_type, CommandType::ApplyPatch);
        assert!(
            parsed.commands[2].arguments["patch"]
                .as_str()
                .is_some_and(|patch| patch.contains("*** End Patch"))
        );
    }

    #[test]
    fn hpatch_end_inside_heredoc_does_not_close_the_operation() {
        let parsed =
            parse_command_stack("hpatch\nin sample.txt\nrsel 1:1\ntype <<PATCH\nend\nPATCH\nend")
                .expect("parse hpatch heredoc");

        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0].command_type, CommandType::Hpatch);
        assert_eq!(
            parsed.commands[0].arguments["script"].as_str(),
            Some("in sample.txt\nrsel 1:1\ntype <<PATCH\nend\nPATCH")
        );
    }

    #[test]
    fn parses_crlf_and_multiline_json() {
        let parsed = parse_command_stack(
            "exec_command step=2\r\n{\r\n  \"cmd\": \"echo ok\",\r\n  \"tty\": false\r\n}\r\nend\r\n",
        )
        .expect("parse CRLF command stack");

        assert_eq!(parsed.commands[0].step, 2);
        assert_eq!(parsed.commands[0].arguments["tty"], false);
    }

    #[test]
    fn rejects_missing_end_and_non_object_arguments() {
        assert!(parse_command_stack("exec_command\n{\"cmd\":\"echo ok\"}").is_err());
        assert!(parse_command_stack("write_stdin\n[]\nend").is_err());
    }
}
