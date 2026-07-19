use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use codex_mcp::McpServerNotification;
use codex_mcp::SendMcpServerNotification;
use codex_protocol::models::ResponseItem;
use futures::FutureExt;
use tracing::warn;

use super::Session;
use crate::context::ContextualUserFragment;
use crate::context::McpNotification;

const MCP_NOTIFICATION_DEDUPE_CAPACITY: usize = 4_096;
const MCP_NOTIFICATION_QUEUE_CAPACITY: usize = 16;
const MCP_NOTIFICATION_BATCH_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
pub(super) struct McpNotificationState {
    delivery_enabled: bool,
    worker_scheduled: bool,
    pending: VecDeque<McpServerNotification>,
    pending_keys: HashSet<String>,
    dedupe_order: VecDeque<String>,
    dedupe_seen: HashSet<String>,
}

#[derive(Debug, PartialEq)]
enum McpNotificationEnqueueOutcome {
    Duplicate,
    Queued {
        schedule_worker: bool,
        dropped_oldest: bool,
    },
}

impl McpNotificationState {
    fn enqueue(&mut self, notification: McpServerNotification) -> McpNotificationEnqueueOutcome {
        if let Some(dedupe_key) = notification_dedupe_key(&notification)
            && !self.mark_seen(dedupe_key)
        {
            return McpNotificationEnqueueOutcome::Duplicate;
        }

        let pending_key = notification_pending_key(&notification);
        if pending_key
            .as_ref()
            .is_some_and(|key| self.pending_keys.contains(key))
        {
            return McpNotificationEnqueueOutcome::Duplicate;
        }

        let dropped_oldest = if self.pending.len() >= MCP_NOTIFICATION_QUEUE_CAPACITY {
            if let Some(dropped) = self.pending.pop_front() {
                if let Some(key) = notification_pending_key(&dropped) {
                    self.pending_keys.remove(&key);
                }
                if let Some(key) = notification_dedupe_key(&dropped) {
                    self.unmark_seen(&key);
                }
            }
            true
        } else {
            false
        };
        if let Some(key) = pending_key {
            self.pending_keys.insert(key);
        }
        self.pending.push_back(notification);

        let schedule_worker = self.delivery_enabled && !self.worker_scheduled;
        if schedule_worker {
            self.worker_scheduled = true;
        }
        McpNotificationEnqueueOutcome::Queued {
            schedule_worker,
            dropped_oldest,
        }
    }

    fn enable_delivery(&mut self) -> bool {
        self.delivery_enabled = true;
        if self.pending.is_empty() || self.worker_scheduled {
            return false;
        }
        self.worker_scheduled = true;
        true
    }

    fn drain_pending(&mut self) -> Vec<McpServerNotification> {
        self.worker_scheduled = false;
        self.pending_keys.clear();
        self.pending.drain(..).collect()
    }

    fn mark_seen(&mut self, key: String) -> bool {
        if !self.dedupe_seen.insert(key.clone()) {
            return false;
        }
        self.dedupe_order.push_back(key);
        while self.dedupe_order.len() > MCP_NOTIFICATION_DEDUPE_CAPACITY {
            if let Some(oldest) = self.dedupe_order.pop_front() {
                self.dedupe_seen.remove(&oldest);
            }
        }
        true
    }

    fn unmark_seen(&mut self, key: &str) {
        if self.dedupe_seen.remove(key) {
            self.dedupe_order.retain(|existing| existing != key);
        }
    }
}

impl Session {
    pub(super) fn new_mcp_server_notification_sender(
        self: &Arc<Self>,
    ) -> SendMcpServerNotification {
        let session = Arc::downgrade(self);
        Arc::new(move |notification| {
            let session = session.clone();
            async move {
                let Some(session) = session.upgrade() else {
                    return Ok(());
                };
                session.enqueue_mcp_server_notification(notification).await
            }
            .boxed()
        })
    }

    pub(super) fn mcp_server_notification_sender(&self) -> SendMcpServerNotification {
        match self.mcp_notification_sender.get() {
            Some(sender) => sender.clone(),
            None => panic!("MCP notification sender should be initialized"),
        }
    }

    pub(super) async fn enable_mcp_server_notification_delivery(self: &Arc<Self>) {
        let schedule_worker = self.mcp_notification_state.lock().await.enable_delivery();
        if schedule_worker {
            self.schedule_mcp_notification_worker();
        }
    }

    async fn enqueue_mcp_server_notification(
        self: &Arc<Self>,
        notification: McpServerNotification,
    ) -> Result<()> {
        let server_name = notification.server_name.clone();
        let method = notification.notification.method.clone();
        let outcome = self
            .mcp_notification_state
            .lock()
            .await
            .enqueue(notification);
        let McpNotificationEnqueueOutcome::Queued {
            schedule_worker,
            dropped_oldest,
        } = outcome
        else {
            return Ok(());
        };
        if dropped_oldest {
            warn!(
                server_name,
                method, "dropping oldest MCP notification because the pending queue is full"
            );
        }
        if schedule_worker {
            self.schedule_mcp_notification_worker();
        }
        Ok(())
    }

    fn schedule_mcp_notification_worker(self: &Arc<Self>) {
        let session = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(MCP_NOTIFICATION_BATCH_DELAY).await;
            let Some(session) = session.upgrade() else {
                return;
            };
            session.flush_mcp_server_notifications().await;
        });
    }

    async fn flush_mcp_server_notifications(self: &Arc<Self>) {
        let notifications = self.mcp_notification_state.lock().await.drain_pending();
        if notifications.is_empty() {
            return;
        }
        let items = notifications
            .into_iter()
            .map(|notification| {
                let McpServerNotification {
                    server_name,
                    notification,
                } = notification;
                ContextualUserFragment::into(McpNotification::new(
                    server_name,
                    notification.method,
                    notification.source,
                    notification.message_id,
                    notification.payload,
                ))
            })
            .collect::<Vec<ResponseItem>>();
        self.deliver_mcp_notification_items(items).await;
    }

    async fn deliver_mcp_notification_items(self: &Arc<Self>, items: Vec<ResponseItem>) {
        let Err(items) = self.inject_if_running(items).await else {
            return;
        };
        match self.try_start_turn_if_idle(items).await {
            Ok(()) => {}
            Err(rejection) => {
                let reason = rejection.reason();
                let items = rejection.into_input();
                let Err(items) = self.inject_if_running(items).await else {
                    return;
                };
                warn!(
                    "recording MCP notifications without starting a turn because idle delivery was rejected: {reason:?}"
                );
                self.inject_no_new_turn(items, None).await;
            }
        }
    }
}

fn notification_dedupe_key(notification: &McpServerNotification) -> Option<String> {
    let message_id = notification.notification.message_id.as_deref()?;
    serde_json::to_string(&(
        notification.server_name.as_str(),
        notification.notification.method.as_str(),
        notification.notification.source.as_deref(),
        message_id,
    ))
    .ok()
}

fn notification_pending_key(notification: &McpServerNotification) -> Option<String> {
    if notification.notification.message_id.is_some() {
        return None;
    }
    serde_json::to_string(&(
        notification.server_name.as_str(),
        notification.notification.method.as_str(),
        notification.notification.source.as_deref(),
        &notification.notification.payload,
    ))
    .ok()
}

#[cfg(test)]
#[path = "mcp_notifications_tests.rs"]
mod tests;
