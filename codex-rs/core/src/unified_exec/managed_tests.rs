use super::*;
use pretty_assertions::assert_eq;

#[tokio::test(start_paused = true)]
async fn managed_wait_backoff_resets_after_a_gap_or_output() {
    let mut state = JobState::default();
    let mut waits = Vec::new();
    for _ in 0..6 {
        waits.push(state.next_poll_wait(1));
        state.last_poll = Some(Instant::now());
    }
    assert_eq!(waits, vec![5_000, 10_000, 30_000, 60_000, 300_000, 300_000]);
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(state.next_poll_wait(1), 5_000);
    state.last_poll = None;
    assert_eq!(state.next_poll_wait(20_000), 20_000);
}

#[tokio::test]
async fn managed_admission_is_bounded_even_before_registration() {
    let manager = UnifiedExecProcessManager::default();
    let mut reservations = Vec::new();
    for _ in 0..RETAINED_RESULTS + super::super::MAX_UNIFIED_EXEC_PROCESSES {
        reservations.push(manager.reserve_managed_command().await.unwrap());
    }
    assert!(manager.reserve_managed_command().await.is_err());
    reservations.pop();
    assert!(manager.reserve_managed_command().await.is_ok());
}

#[tokio::test]
async fn cancelled_managed_reader_wakes_the_parked_turn_after_unlocking() {
    let changed = Arc::new(Notify::new());
    let state = Arc::new(Mutex::new(JobState::default()));
    let notified = changed.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let reader_state = Arc::clone(&state);
    let reader_changed = Arc::clone(&changed);
    let reader = tokio::spawn(async move {
        let _finished = NotifyOnDrop(reader_changed);
        let _state = reader_state.lock_owned().await;
        locked_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    locked_rx.await.unwrap();
    reader.abort();
    assert!(reader.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), notified)
        .await
        .unwrap();
    assert!(state.try_lock().is_ok());
}
