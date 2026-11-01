//! Existing subscriber/idle unload countdown, including opted-in registry work.
use super::thread_lifecycle::ListenerTaskContext;
use super::*;

pub(super) struct UnloadingState {
    conversation: std::sync::Weak<CodexThread>,
    thread_id: ThreadId,
    outgoing: Arc<OutgoingMessageSender>,
    thread_state_manager: ThreadStateManager,
    has_background_work: bool,
    job_timers: Vec<codex_app_server_protocol::CodexdcBackgroundJobTimer>,
    delay: Duration,
    has_subscribers_rx: watch::Receiver<bool>,
    has_subscribers: (bool, Instant),
    thread_status_rx: watch::Receiver<ThreadStatus>,
    is_active: (bool, Instant),
}

impl UnloadingState {
    pub(super) async fn new(
        listener_task_context: &ListenerTaskContext,
        thread_id: ThreadId,
        conversation: Arc<CodexThread>,
        delay: Duration,
        has_subscribers_rx: watch::Receiver<bool>,
        thread_status_rx: watch::Receiver<ThreadStatus>,
    ) -> Self {
        let has_subscribers = (*has_subscribers_rx.borrow(), Instant::now());
        let has_background_work = conversation.has_live_background_work().await;
        let is_active = (
            matches!(*thread_status_rx.borrow(), ThreadStatus::Active { .. })
                || has_background_work,
            Instant::now(),
        );
        Self {
            conversation: Arc::downgrade(&conversation),
            thread_id,
            outgoing: Arc::clone(&listener_task_context.outgoing),
            thread_state_manager: listener_task_context.thread_state_manager.clone(),
            has_background_work,
            job_timers: Vec::new(),
            delay,
            has_subscribers_rx,
            has_subscribers,
            thread_status_rx,
            is_active,
        }
    }

    fn unloading_target(&self) -> Option<Instant> {
        match (self.has_subscribers, self.is_active) {
            ((false, has_no_subscribers_since), (false, is_inactive_since)) => {
                std::cmp::max(has_no_subscribers_since, is_inactive_since).checked_add(self.delay)
            }
            _ => None,
        }
    }

    fn sync_receiver_values(&mut self) {
        let has_subscribers = *self.has_subscribers_rx.borrow();
        if self.has_subscribers.0 != has_subscribers {
            self.has_subscribers = (has_subscribers, Instant::now());
        }

        let is_active = matches!(*self.thread_status_rx.borrow(), ThreadStatus::Active { .. })
            || self.has_background_work;
        if self.is_active.0 != is_active {
            self.is_active = (is_active, Instant::now());
        }
    }

    pub(super) fn should_unload_now(&mut self) -> bool {
        self.sync_receiver_values();
        self.unloading_target()
            .is_some_and(|target| target <= Instant::now())
    }

    pub(super) fn note_thread_activity_observed(&mut self) {
        if !self.is_active.0 {
            self.is_active = (false, Instant::now());
        }
    }

    pub(super) async fn refresh_background_work(
        &mut self,
    ) -> Option<codex_app_server_protocol::CodexdcBackgroundWorkState> {
        // Shutdown relays outlive their runtime and must not keep it resident.
        let Some(conversation) = self.conversation.upgrade() else {
            self.has_background_work = false;
            self.sync_receiver_values();
            return None;
        };
        let core = conversation.background_work_state().await;
        let pending = core.pending;
        let state = codex_app_server_protocol::CodexdcBackgroundWorkState {
            background_jobs_enabled: conversation.background_jobs_enabled(),
            job_timers: core
                .job_timers
                .into_iter()
                .map(
                    |timer| codex_app_server_protocol::CodexdcBackgroundJobTimer {
                        item_id: timer.item_id,
                        process_id: timer.process_id,
                        started_at_ms: timer.started_at_ms,
                        explicit_timeout_seconds: timer.explicit_timeout_seconds,
                        duration_seconds: timer.duration_seconds,
                    },
                )
                .collect(),
            thread_id: self.thread_id.to_string(),
            pending,
            active: core.active,
            last_turn_id: core.last_turn_id,
            idle_continuation: core.idle_continuation,
            unattended_deadline_ms: core.unattended_deadline.as_millis() as u64,
        };
        if (pending != self.has_background_work || state.job_timers != self.job_timers)
            && state.background_jobs_enabled
        {
            let connections = self
                .thread_state_manager
                .subscribed_connection_ids(self.thread_id)
                .await;
            ThreadScopedOutgoingMessageSender::new(
                Arc::clone(&self.outgoing),
                connections,
                self.thread_id,
            )
            .send_server_notification(ServerNotification::CodexdcBackgroundWorkChanged(
                state.clone(),
            ))
            .await;
        }
        self.job_timers = state.job_timers.clone();
        self.has_background_work = pending;
        self.sync_receiver_values();
        Some(state)
    }

    pub(super) async fn wait_for_unloading_trigger(&mut self) -> bool {
        loop {
            // Register before reading the predicate so a drain/revoke cannot
            // disappear between that read and the existing unload wait.
            let background_changed = self
                .conversation
                .upgrade()
                .map(|conversation| conversation.background_work_changed());
            tokio::pin!(background_changed);
            if let Some(changed) = background_changed.as_mut().as_pin_mut() {
                changed.enable();
            }
            self.refresh_background_work().await;
            let unloading_target = self.unloading_target();
            if let Some(target) = unloading_target
                && target <= Instant::now()
            {
                tokio::task::yield_now().await;
                return true;
            }
            let unloading_sleep = async {
                if let Some(target) = unloading_target {
                    tokio::time::sleep_until(target.into()).await;
                } else {
                    futures::future::pending::<()>().await;
                }
            };
            tokio::select! {
                _ = unloading_sleep => return true,
                _ = async {
                    match background_changed.as_mut().as_pin_mut() {
                        Some(changed) => changed.await,
                        None => futures::future::pending().await,
                    }
                } => {},
                changed = self.has_subscribers_rx.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                    self.sync_receiver_values();
                },
                changed = self.thread_status_rx.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                    self.sync_receiver_values();
                },
            }
        }
    }
}
