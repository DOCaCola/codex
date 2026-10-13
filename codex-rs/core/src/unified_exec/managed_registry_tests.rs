#![allow(clippy::unwrap_used)]

#[path = "managed_interrupt_tests.rs"]
mod interrupt_tests;

use super::*;
use crate::session::step_context::StepContext;
use crate::session::tests::make_session_and_context;
use crate::unified_exec::ExecCommandRequest;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecProcess;
use crate::unified_exec::UnifiedExecProcessManager;
use crate::unified_exec::managed::Execution;
use crate::unified_exec::managed::ExecutionMode;
use crate::unified_exec::managed::JobState;
use crate::unified_exec::managed::metadata::JobMetadata;
use pretty_assertions::assert_eq;
use pretty_assertions::assert_ne;
use std::sync::atomic::AtomicBool;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

async fn exited_process() -> Arc<UnifiedExecProcess> {
    let (writer_tx, _writer_rx) = tokio::sync::mpsc::channel(1);
    let (_stdout_tx, stdout_rx) = tokio::sync::broadcast::channel(1);
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    exit_tx.send(0).unwrap();
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
            Box::new(crate::unified_exec::NoopSpawnLifecycle),
        )
        .await
        .unwrap(),
    );
    process.cancellation_token().cancelled().await;
    process
}

fn job(context: &UnifiedExecContext, process: Arc<UnifiedExecProcess>) -> Arc<Job> {
    let turn = &context.step_context.turn;
    let request = ExecCommandRequest {
        command_stack: None,
        managed: None,
        command: vec!["echo".into()],
        shell: crate::shell::ShellInvocation {
            shell: crate::shell::Shell {
                shell_type: crate::shell::ShellType::Sh,
                shell_path: "sh".into(),
            },
            use_login_shell: true,
        },
        hook_command: "echo".into(),
        process_id: 1000,
        yield_time_ms: 1,
        yield_source: crate::unified_exec::background_policy::YieldSource::Legacy,
        max_output_tokens: None,
        #[allow(deprecated)]
        cwd: turn.cwd.clone().into(),
        #[allow(deprecated)]
        sandbox_cwd: turn.cwd.clone().into(),
        turn_environment: turn.initial_environments.primary().cloned().unwrap(),
        shell_mode: codex_tools::UnifiedExecShellMode::Direct,
        network: None,
        tty: false,
        sandbox_permissions: crate::sandboxing::SandboxPermissions::UseDefault,
        additional_permissions: None,
        additional_permissions_preapproved: false,
        justification: None,
        prefix_rule: None,
    };
    Arc::new(Job {
        background_jobs: turn
            .config
            .features
            .enabled(codex_features::Feature::BackgroundJobs),
        origin: Arc::clone(&context.step_context),
        metadata: Mutex::new(JobMetadata::new(
            &request,
            context,
            Execution {
                mode: ExecutionMode::Background,
                timeout: context
                    .session
                    .background_jobs
                    .deadline(ExecutionMode::Background, /*explicit*/ None),
                explicit_timeout: None,
            },
            process.as_ref(),
            tokio::time::Instant::now(),
            context
                .session
                .background_jobs
                .deadline(ExecutionMode::Background, /*explicit*/ None),
        )),
        _slot: Arc::new(Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap()),
        process,
        initial_response_done: CancellationToken::new(),
        initial_response_consumed: AtomicBool::new(false),
        acknowledged: AtomicBool::new(false),
        revoked: CancellationToken::new(),
        state: Arc::new(Mutex::new(JobState::default())),
    })
}

async fn background_context() -> UnifiedExecContext {
    let (mut session, mut turn) = make_session_and_context().await;
    Arc::make_mut(&mut turn.config)
        .features
        .enable(codex_features::Feature::BackgroundJobs)
        .unwrap();
    session.background_jobs = crate::unified_exec::background_policy::BackgroundJobPolicy::capture(
        &turn.config,
        &turn.session_source,
    );
    UnifiedExecContext::new(
        Arc::new(session),
        StepContext::for_test(Arc::new(turn)),
        CancellationToken::new(),
        "call".into(),
    )
}

#[tokio::test]
async fn background_job_queue_process_id_reuse_rejects_retired_completion_and_watcher() {
    let context = background_context().await;
    let manager = UnifiedExecProcessManager::default();
    let registry = &manager.managed.registry;
    let old_process = exited_process().await;
    let old = job(&context, Arc::clone(&old_process));
    let old_id = old.metadata.lock().await.job_id;
    registry
        .insert(/*process_id*/ 1000, old_id, Arc::clone(&old))
        .await;
    registry.enqueue(old_id).await;
    old.acknowledged.store(/*val*/ true, Ordering::Release);
    for id in 1001..1064 {
        let retired = job(&context, Arc::clone(&old_process));
        retired.acknowledged.store(/*val*/ true, Ordering::Release);
        let job_id = retired.metadata.lock().await.job_id;
        registry.insert(id, job_id, retired).await;
    }
    registry.evict_retired().await;
    assert!(registry.jobs().await.is_empty());
    let replacement = job(&context, exited_process().await);
    let replacement_id = replacement.metadata.lock().await.job_id;
    assert_ne!(old_id, replacement_id);
    registry
        .insert(1000, replacement_id, Arc::clone(&replacement))
        .await;
    registry.enqueue(old_id).await;
    assert!(registry.queued().await.is_empty());
    manager
        .finalize_managed_outcome(/*process_id*/ 1000, &old_process)
        .await;
    assert!(replacement.metadata.lock().await.terminal_state.is_none());
    manager
        .finalize_managed_outcome(/*process_id*/ 1000, &replacement.process)
        .await;
    registry.enqueue(replacement_id).await;
    let queued = registry.queued().await;
    assert_eq!(queued.len(), 1);
    assert!(Arc::ptr_eq(&queued[0].1, &replacement));
    registry.consumed(replacement_id).await;
    registry.enqueue(replacement_id).await;
    assert!(registry.queued().await.is_empty());
}

#[tokio::test]
async fn background_job_queue_claim_keeps_process_reserved_during_revocation() {
    let context = background_context().await;
    let registry = SessionJobRegistry::default();
    let process = exited_process().await;
    let claimed = job(&context, Arc::clone(&process));
    let job_id = claimed.metadata.lock().await.job_id;
    registry
        .insert(/*process_id*/ 1000, job_id, Arc::clone(&claimed))
        .await;
    let _claim = Arc::clone(&claimed.state).lock_owned().await;
    claimed.revoked.cancel();
    for id in 1001..1064 {
        let retired = job(&context, Arc::clone(&process));
        retired.acknowledged.store(/*val*/ true, Ordering::Release);
        let job_id = retired.metadata.lock().await.job_id;
        registry.insert(id, job_id, retired).await;
    }
    registry.evict_retired().await;
    assert_eq!(registry.jobs().await.len(), 1);
    assert!(registry.contains(job_id).await);
    assert!(Arc::ptr_eq(&registry.get(1000).await.unwrap(), &claimed));
}

#[tokio::test]
async fn background_job_queue_parks_until_exit_publication() {
    let context = background_context().await;
    let manager = UnifiedExecProcessManager::default();
    let finished = job(&context, exited_process().await);
    finished.initial_response_done.cancel();
    let job_id = finished.metadata.lock().await.job_id;
    manager
        .managed
        .registry
        .insert(/*process_id*/ 1000, job_id, finished)
        .await;
    let waiting = manager.wait_for_managed_commands(&context);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting)
            .await
            .is_err()
    );
    manager.managed.registry.enqueue(job_id).await;
    manager.managed.changed.notify_waiters();
    assert!(waiting.await);
}

fn cached_completion_output() -> crate::tools::context::ExecCommandToolOutput {
    crate::tools::context::ExecCommandToolOutput {
        completion_delivery: Some(true),
        event_call_id: "call".into(),
        chunk_id: "chunk".into(),
        wall_time: std::time::Duration::from_secs(1),
        raw_output: b"retained-result".to_vec(),
        truncation_policy: codex_utils_output_truncation::TruncationPolicy::Bytes(1024),
        max_output_tokens: None,
        process_id: None,
        exit_code: Some(0),
        original_token_count: None,
        output_omitted_bytes: None,
        hook_command: None,
    }
}

// Output already accepted before interrupt stays pending and
// is committed by the next user context without reading a retired terminal.
#[tokio::test]
async fn background_jobs_interrupted_cached_completion_survives_until_commit() {
    let context = background_context().await;
    let manager = UnifiedExecProcessManager::default();
    let completed = job(&context, exited_process().await);
    completed.initial_response_done.cancel();
    completed.state.lock().await.result = Some(cached_completion_output());
    let job_id = completed.metadata.lock().await.job_id;
    manager
        .managed
        .registry
        .insert(/*process_id*/ 1000, job_id, Arc::clone(&completed))
        .await;
    manager.managed.registry.enqueue(job_id).await;
    context.cancellation_token.cancel();
    assert!(!manager.record_command_completions(&context).await);
    assert!(!completed.acknowledged.load(Ordering::Acquire));
    assert_eq!(manager.managed.registry.queued().await.len(), 1);
    let next = UnifiedExecContext::new(
        Arc::clone(&context.session),
        Arc::clone(&context.step_context),
        CancellationToken::new(),
        "next-user".into(),
    );
    assert!(manager.record_command_completions(&next).await);
    assert!(completed.acknowledged.load(Ordering::Acquire));
    assert!(manager.managed.registry.queued().await.is_empty());
    assert!(!manager.record_command_completions(&next).await);
}

// A stopped cached result stays revoked even if explicitly read.
#[tokio::test]
async fn background_jobs_stopped_cached_completion_cannot_be_acknowledged() {
    let context = background_context().await;
    let manager = UnifiedExecProcessManager::default();
    let completed = job(&context, exited_process().await);
    completed.initial_response_done.cancel();
    completed.state.lock().await.result = Some(cached_completion_output());
    let job_id = completed.metadata.lock().await.job_id;
    manager
        .managed
        .registry
        .insert(/*process_id*/ 1000, job_id, Arc::clone(&completed))
        .await;
    manager.managed.registry.enqueue(job_id).await;
    assert!(
        manager
            .stop_managed_command(/*process_id*/ 1000, &completed.process)
            .await
            .unwrap()
    );
    let result = manager
        .read_managed_command(
            &context,
            crate::unified_exec::WriteStdinRequest {
                process_id: 1000,
                input: "",
                yield_time_ms: 5000,
                max_output_tokens: None,
                truncation_policy: codex_utils_output_truncation::TruncationPolicy::Bytes(1024),
                interaction_event: None,
            },
            crate::unified_exec::managed::WaitMode::Completion,
        )
        .await
        .unwrap();
    assert_eq!(
        (result.raw_output, result.completion_delivery),
        (b"retained-result".to_vec(), Some(false))
    );
    assert!(!completed.acknowledged.load(Ordering::Acquire));
    assert!(!manager.record_command_completions(&context).await);
    assert!(manager.managed.registry.queued().await.is_empty());
}

// Exit before queue publication must not create an unload gap.
#[tokio::test]
async fn background_jobs_lifecycle_guard_covers_exit_handoff_and_consumption() {
    let context = background_context().await;
    let manager = UnifiedExecProcessManager::default();
    let completed = job(&context, exited_process().await);
    let job_id = completed.metadata.lock().await.job_id;
    manager
        .managed
        .registry
        .insert(/*process_id*/ 1000, job_id, Arc::clone(&completed))
        .await;
    assert!(manager.has_live_background_work().await);
    manager.managed.registry.enqueue(job_id).await;
    assert!(manager.has_live_background_work().await);
    completed
        .acknowledged
        .store(/*val*/ true, Ordering::Release);
    assert!(!manager.has_live_background_work().await);
}

// Manual/revoked results and the flag-off map do not hold idle threads.
#[tokio::test]
async fn background_jobs_lifecycle_guard_excludes_manual_revoked_and_flag_off_records() {
    let context = background_context().await;
    let manager = UnifiedExecProcessManager::default();
    let completed = job(&context, exited_process().await);
    completed.metadata.lock().await.mode = ExecutionMode::Foreground;
    let job_id = completed.metadata.lock().await.job_id;
    manager
        .managed
        .registry
        .insert(/*process_id*/ 1000, job_id, Arc::clone(&completed))
        .await;
    assert!(!manager.has_live_background_work().await);
    completed.metadata.lock().await.mode = ExecutionMode::Background;
    assert!(manager.has_live_background_work().await);
    completed.revoked.cancel();
    assert!(!manager.has_live_background_work().await);
    let (session, turn) = make_session_and_context().await;
    let legacy_context = UnifiedExecContext::new(
        Arc::new(session),
        StepContext::for_test(Arc::new(turn)),
        CancellationToken::new(),
        "legacy".into(),
    );
    let legacy = job(&legacy_context, exited_process().await);
    assert!(!legacy.background_jobs);
    manager.managed.jobs.lock().await.insert(1001, legacy);
    assert!(!manager.has_live_background_work().await);
}

// A reused terminal handle must not hide the older pending UUID.
#[tokio::test]
async fn background_jobs_lifecycle_guard_retains_pending_result_after_process_id_reuse() {
    enum Disposition {
        Acknowledged,
        Consumed,
        Revoked,
    }
    for disposition in [
        Disposition::Acknowledged,
        Disposition::Consumed,
        Disposition::Revoked,
    ] {
        let context = background_context().await;
        let manager = UnifiedExecProcessManager::default();
        let older = job(&context, exited_process().await);
        older.metadata.lock().await.mode = ExecutionMode::Auto;
        older.state.lock().await.result = Some(cached_completion_output());
        let older_id = older.metadata.lock().await.job_id;
        manager
            .managed
            .registry
            .insert(/*process_id*/ 1000, older_id, Arc::clone(&older))
            .await;
        manager.managed.registry.enqueue(older_id).await;
        let replacement = job(&context, exited_process().await);
        replacement
            .acknowledged
            .store(/*val*/ true, Ordering::Release);
        let replacement_id = replacement.metadata.lock().await.job_id;
        manager
            .managed
            .registry
            .insert(
                /*process_id*/ 1000,
                replacement_id,
                Arc::clone(&replacement),
            )
            .await;
        let indexed = manager.managed.registry.jobs().await;
        assert_eq!(indexed.len(), 1);
        assert!(Arc::ptr_eq(&indexed[0].1, &replacement));
        assert_eq!(manager.managed.registry.all_jobs().await.len(), 2);
        assert!(manager.has_live_background_work().await);
        match disposition {
            Disposition::Acknowledged => {
                older.acknowledged.store(/*val*/ true, Ordering::Release)
            }
            Disposition::Consumed => older
                .initial_response_consumed
                .store(/*val*/ true, Ordering::Release),
            Disposition::Revoked => older.revoked.cancel(),
        }
        assert!(!manager.has_live_background_work().await);
    }
}

// Ready output requests another sample in this turn, leaving no idle wake.
#[tokio::test]
async fn idle_continuation_delivers_ready_completion_at_turn_end_without_a_wake() {
    let mut context = background_context().await;
    Arc::get_mut(&mut context.session)
        .unwrap()
        .background_continuation =
        crate::session::background_continuation::BackgroundContinuation::new(/*enabled*/ true);
    let manager = UnifiedExecProcessManager::default();
    let completed = job(&context, exited_process().await);
    completed.initial_response_done.cancel();
    completed.state.lock().await.result = Some(cached_completion_output());
    let job_id = completed.metadata.lock().await.job_id;
    manager
        .managed
        .registry
        .insert(/*process_id*/ 1000, job_id, Arc::clone(&completed))
        .await;
    manager.managed.registry.enqueue(job_id).await;
    let previous_turn = context.session.background_work_state().await.last_turn_id;
    assert!(
        manager
            .settle_managed_commands(&context, /*model_needs_follow_up*/ false)
            .await
    );
    assert!(completed.acknowledged.load(Ordering::Acquire));
    assert!(manager.background_wake_candidates().await.is_empty());
    assert_eq!(
        context.session.background_work_state().await.last_turn_id,
        previous_turn
    );
    assert!(
        !manager
            .settle_managed_commands(&context, /*model_needs_follow_up*/ false)
            .await
    );
}

// Constructor records unlimited versus host-default provenance.
#[tokio::test]
async fn background_metadata_records_host_default_without_explicit_timeout_label() {
    let mut context = background_context().await;
    Arc::get_mut(&mut context.session)
        .unwrap()
        .background_jobs
        .host = crate::unified_exec::background_policy::HostClass::Interactive;
    let unlimited = job(&context, exited_process().await);
    assert!(unlimited.metadata.lock().await.deadline_source.is_none());
    Arc::get_mut(&mut context.session)
        .unwrap()
        .background_jobs
        .host = crate::unified_exec::background_policy::HostClass::Unattended;
    let timed = job(&context, exited_process().await);
    let metadata = timed.metadata.lock().await;
    assert!(metadata.effective_deadline.is_some());
    assert!(metadata.explicit_timeout_ms.is_none());
    assert_eq!(
        metadata.deadline_source,
        Some(crate::unified_exec::managed::metadata::DeadlineSource::HostDefault)
    );
    assert_eq!(metadata.deadline_origin, Some("process_start"));
}
