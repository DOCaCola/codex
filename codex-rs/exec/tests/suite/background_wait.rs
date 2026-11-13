//! Real exec protocol coverage across idle-continuation and output modes.
use core_test_support::responses::*;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex_exec::test_codex_exec;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_waits_for_background_completion_and_reports_last_turn() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    for (idle, json_output) in [(false, false), (false, true), (true, false), (true, true)] {
        let test = test_codex_exec();
        let server = start_mock_server().await;
        let schema = json!({"type":"object","properties":{"answer":{"type":"string"}},
            "required":["answer"],"additionalProperties":false});
        let schema_path = test.cwd_path().join("schema.json");
        let last_path = test.cwd_path().join("last.txt");
        std::fs::write(&schema_path, schema.to_string())?;
        let cmd = if cfg!(windows) {
            "Set-Content -Path started -Value ready; while (!(Test-Path release)) { Start-Sleep -Milliseconds 50 }; Write-Output completed-marker"
        } else {
            "touch started; while [ ! -e release ]; do sleep 0.05; done; echo completed-marker"
        };
        let mut args = json!({"cmd":cmd,"execution_mode":"background"});
        if cfg!(windows) {
            args["shell"] = json!("powershell");
        }
        let responses = vec![
            sse(vec![
                ev_function_call("launch", "exec_command", &args.to_string()),
                ev_completed("launch"),
            ]),
            sse(vec![
                ev_assistant_message("waiting", "{\"answer\":\"waiting\"}"),
                ev_completed("waiting"),
            ]),
            sse(vec![
                ev_assistant_message("final", "{\"answer\":\"final\"}"),
                ev_completed("final"),
            ]),
        ];
        let log = mount_sse_sequence(&server, responses).await;
        let mut command = test.cmd_with_server(&server);
        command
            .args([
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
                "-m",
                "gpt-5.5",
                "-c",
                "features.background_jobs=true",
                "-c",
                if idle {
                    "features.background_jobs_idle_continuation=true"
                } else {
                    "features.background_jobs_idle_continuation=false"
                },
                "-c",
                "features.unified_exec=true",
                "-c",
                "features.command_stack=false",
                "-c",
                "features.code_mode=false",
                "-c",
                "features.enable_request_compression=false",
                "-c",
                "background_jobs.unattended_deadline_ms=30000",
                "--output-schema",
            ])
            .arg(&schema_path)
            .arg("--output-last-message")
            .arg(&last_path);
        if json_output {
            command.arg("--json");
        }
        command
            .arg("launch the command")
            .env("RUST_LOG", "codex_app_server::outgoing_message=trace");
        let mut process = tokio::process::Command::new(command.get_program());
        process
            .args(command.get_args())
            .envs(
                command
                    .get_envs()
                    .filter_map(|(key, value)| value.map(|value| (key, value))),
            )
            .current_dir(test.cwd_path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = process.spawn()?;
        let ready = Arc::new(tokio::sync::Notify::new());
        let signal = Arc::clone(&ready);
        let stderr = child.stderr.take().unwrap();
        let reader = tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            let mut text = String::new();
            let mut initial_complete = false;
            while let Some(line) = lines.next_line().await? {
                if !initial_complete && line.contains("app-server event: turn/completed") {
                    initial_complete = true;
                    signal.notify_one();
                }
                text.push_str(&line);
                text.push('\n');
            }
            anyhow::Ok(text)
        });
        if idle {
            tokio::time::timeout(Duration::from_secs(20), ready.notified()).await?;
            assert!(
                child.try_wait()?.is_none(),
                "exec must survive its initial completion"
            );
            assert!(
                std::fs::read_to_string(&last_path)?.is_empty(),
                "last-message file remains empty until final shutdown"
            );
        } else {
            tokio::time::timeout(Duration::from_secs(20), async {
                while !test.cwd_path().join("started").exists() || log.requests().len() < 2 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await?;
            assert!(
                child.try_wait()?.is_none(),
                "the flag-off turn must remain parked"
            );
            assert!(
                std::fs::read_to_string(&last_path)?.is_empty(),
                "a parked turn cannot emit final output yet"
            );
        }
        std::fs::write(test.cwd_path().join("release"), "go")?;
        let output =
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output()).await??;
        let stderr = reader.await??;
        assert!(output.status.success(), "{stderr}");
        let last = "{\"answer\":\"final\"}";
        assert_eq!(std::fs::read_to_string(last_path)?, last);
        assert_eq!(log.requests().len(), 3);
        assert_eq!(
            log.last_request().unwrap().body_json()["text"]["format"]["schema"],
            schema
        );
        let stdout = String::from_utf8(output.stdout)?;
        if json_output {
            assert_eq!(
                stdout.matches("\"type\":\"turn.started\"").count(),
                if idle { 2 } else { 1 }
            );
            assert_eq!(
                stdout.matches("\"type\":\"turn.completed\"").count(),
                if idle { 2 } else { 1 }
            );
            assert!(!stdout.contains("continuing automatically"));
        } else {
            assert!(stdout.contains(last));
            assert_eq!(
                stderr
                    .matches("Background job completed; continuing automatically.")
                    .count(),
                usize::from(idle)
            );
            assert_eq!(stderr.matches("tokens used").count(), 1);
        }
    }
    Ok(())
}
