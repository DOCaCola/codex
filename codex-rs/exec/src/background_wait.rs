//! Protocol-driven lifetime for exec's primary thread; no job polling.
use std::collections::HashSet;
use std::time::Duration;

use codex_app_server_protocol::CodexdcBackgroundWorkState;
use codex_app_server_protocol::ServerNotification;
use tokio::time::Instant;

pub(crate) struct BackgroundWait {
    pub enabled: bool,
    pub current: Option<String>,
    pub failed: bool,
    interrupted: bool,
    pending: bool,
    server_active: bool,
    observed_turn: Option<String>,
    completed: HashSet<String>,
    last_completed: Option<String>,
    bound: Duration,
    deadline: Option<Instant>,
}

impl BackgroundWait {
    pub(crate) fn new(enabled: bool, initial: String, bound: Duration) -> Self {
        Self {
            enabled, current: Some(initial), bound, failed: false, interrupted: false,
            pending: true, server_active: true, observed_turn: None,
            completed: HashSet::new(), last_completed: None, deadline: None,
        }
    }

    pub(crate) fn adopt(&mut self, notification: &ServerNotification, primary: &str) -> bool {
        if let ServerNotification::TurnStarted(start) = notification
            && self.enabled && !self.interrupted && self.current.is_none()
            && start.thread_id == primary && !self.completed.contains(&start.turn.id)
        {
            self.current = Some(start.turn.id.clone());
            self.observed_turn = self.current.clone();
            self.server_active = true;
            self.deadline = None;
            return true;
        }
        false
    }

    pub(crate) fn complete(&mut self, failed: bool) {
        self.failed |= failed;
        let turn = self.current.take().expect("filtered completion owns the tracked turn");
        self.completed.insert(turn.clone());
        self.last_completed = Some(turn);
        self.deadline = self.enabled.then(|| Instant::now() + self.bound);
    }

    pub(crate) fn observe(&mut self, state: &CodexdcBackgroundWorkState) {
        // Read replies can overtake queued notifications. Never regress to an older turn.
        if state.last_turn_id.as_ref().is_some_and(|turn| self.completed.contains(turn))
            && (state.last_turn_id != self.last_completed
                || self.observed_turn.as_ref().is_some_and(|turn| {
                    !self.completed.contains(turn) && self.current.is_none()
                }))
        {
            return;
        }
        self.pending = state.pending;
        self.server_active = state.active;
        self.observed_turn = state.last_turn_id.clone();
    }

    pub(crate) fn finished(&self) -> bool {
        self.current.is_none()
            && (!self.enabled || self.interrupted
                || (!self.pending && !self.server_active && self.observed_turn == self.last_completed))
    }

    pub(crate) fn interrupt(&mut self) {
        self.failed = true;
        self.interrupted = true;
    }

    pub(crate) fn expire(&mut self) {
        self.interrupt();
    }

    pub(crate) async fn wait_for_bound(&self) {
        if let Some(deadline) = self.deadline.filter(|_| self.enabled && !self.interrupted && self.current.is_none()) {
            tokio::time::sleep_until(deadline).await;
        } else {
            std::future::pending::<()>().await;
        }
    }

    pub(crate) async fn read(
        &mut self,
        client: &codex_app_server_client::InProcessAppServerClient,
        ids: &mut super::RequestIdSequencer,
        thread_id: &str,
    ) -> Result<(), String> {
        let request = super::send_request_with_response::<CodexdcBackgroundWorkState>(client,
            codex_app_server_protocol::ClientRequest::CodexdcBackgroundWorkRead {
                request_id: ids.next(),
                params: codex_app_server_protocol::CodexdcBackgroundJobsListParams { thread_id: thread_id.to_owned() },
            }, "codexdc/backgroundWork/read",
        );
        let state = tokio::time::timeout_at(self.deadline.expect("read starts an idle wait"), request)
            .await.map_err(|_| "background job continuation wait exceeded the unattended deadline".to_owned())??;
        self.observe(&state);
        Ok(())
    }
}

#[cfg(test)]
#[path = "background_wait_tests.rs"]
mod tests;
