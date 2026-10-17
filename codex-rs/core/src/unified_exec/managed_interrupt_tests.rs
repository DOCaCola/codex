//! Older running job identities stay suppressed until regular user execution resumes.
use super::*;

#[tokio::test]
async fn older_running_job_is_suppressed_by_later_turn_and_idle_interrupts() {
    for origin in [Some("later-user-turn"), None] {
        let mut context = background_context().await;
        Arc::get_mut(&mut context.session)
            .unwrap()
            .background_continuation =
            crate::session::background_continuation::BackgroundContinuation::new(
                /*enabled*/ true,
            );
        let (writer_tx, _writer_rx) = tokio::sync::mpsc::channel(1);
        let (_stdout_tx, stdout_rx) = tokio::sync::broadcast::channel(1);
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
        let spawned = codex_utils_pty::spawn_from_driver(codex_utils_pty::ProcessDriver {
            writer_tx,
            stdout_rx,
            stderr_rx: None,
            exit_rx,
            terminator: None,
            writer_handle: None,
            resizer: None,
            #[cfg(windows)]
            tty: false,
        });
        let process = Arc::new(
            UnifiedExecProcess::from_spawned(
                spawned,
                codex_sandboxing::SandboxType::None,
            )
            .await
            .unwrap(),
        );
        assert!(!process.has_exited());
        let record = job(&context, Arc::clone(&process));
        let id = {
            let mut metadata = record.metadata.lock().await;
            metadata.originating_turn = "older-completed-turn".into();
            metadata.job_id
        };
        record.initial_response_done.cancel();
        let manager = &context.session.services.unified_exec_manager;
        manager
            .managed
            .registry
            .insert(/*process_id*/ 1000, id, Arc::clone(&record))
            .await;
        assert!(manager.background_wake_candidates().await.is_empty());
        let guard = context
            .session
            .begin_background_interrupt(
                Some(&codex_protocol::protocol::TurnAbortReason::Interrupted),
                origin,
            )
            .await
            .unwrap();
        drop(guard);
        assert!(
            !context
                .session
                .background_completion_deliverable(id, "older-completed-turn")
                .await
        );
        exit_tx.send(0).unwrap();
        process.cancellation_token().cancelled().await;
        record.state.lock().await.result = Some(cached_completion_output());
        manager.managed.registry.enqueue(id).await;
        assert!(manager.background_wake_candidates().await[0].ready);
        assert!(!context.session.background_completion_ready().await);
        context
            .step_context
            .turn
            .extension_data
            .insert(crate::session::background_continuation::UserStartedTurn);
        context
            .session
            .begin_background_user_turn(&context.step_context.turn)
            .await;
        assert!(
            context
                .session
                .background_completion_deliverable(id, "older-completed-turn")
                .await
        );
        assert!(context.session.background_completion_ready().await);
    }
}
