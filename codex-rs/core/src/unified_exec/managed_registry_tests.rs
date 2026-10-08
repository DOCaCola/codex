#![allow(clippy::unwrap_used)]

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
        shell_type: crate::shell::ShellType::Sh,
        hook_command: "echo".into(),
        process_id: 1000,
        yield_time_ms: 1,
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
        origin: Arc::clone(&context.step_context),
        metadata: Mutex::new(JobMetadata::new(
            &request,
            context,
            Execution {
                mode: ExecutionMode::Background,
                timeout: None,
                explicit_timeout: None,
            },
            tokio::time::Instant::now(),
            /*deadline*/ None,
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
    let (session, mut turn) = make_session_and_context().await;
    Arc::make_mut(&mut turn.config)
        .features
        .enable(codex_features::Feature::BackgroundJobs)
        .unwrap();
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
