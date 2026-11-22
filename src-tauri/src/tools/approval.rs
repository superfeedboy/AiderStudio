use crate::models::ToolApprovalRequestEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, watch, Mutex};
use uuid::Uuid;

#[derive(Default)]
pub struct PendingToolApprovals(pub Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>);

impl PendingToolApprovals {
    pub async fn register(&self, request_id: String) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let mut guard = self.0.lock().await;
        guard.insert(request_id, tx);
        rx
    }

    pub async fn resolve(&self, request_id: &str, approved: bool) -> bool {
        let tx = {
            let mut guard = self.0.lock().await;
            guard.remove(request_id)
        };
        if let Some(sender) = tx {
            sender.send(approved).is_ok()
        } else {
            false
        }
    }

    pub async fn remove(&self, request_id: &str) {
        let mut guard = self.0.lock().await;
        guard.remove(request_id);
    }

    pub async fn await_decision(
        &self,
        request_id: &str,
        rx: oneshot::Receiver<bool>,
        cancel_rx: &mut watch::Receiver<bool>,
        timeout: Duration,
    ) -> Result<bool, String> {
        if *cancel_rx.borrow() {
            self.remove(request_id).await;
            return Err("CANCELLED_BY_USER".to_string());
        }

        tokio::select! {
            biased;
            _ = wait_cancelled(cancel_rx) => {
                self.remove(request_id).await;
                Err("CANCELLED_BY_USER".to_string())
            }
            res = rx => {
                self.remove(request_id).await;
                Ok(res.unwrap_or(false))
            }
            _ = tokio::time::sleep(timeout) => {
                self.remove(request_id).await;
                Ok(false)
            }
        }
    }
}

async fn wait_cancelled(cancel_rx: &mut watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    while cancel_rx.changed().await.is_ok() {
        if *cancel_rx.borrow() {
            return;
        }
    }
    std::future::pending::<()>().await;
}

pub async fn request_tool_approval(
    app: &AppHandle,
    conversation_id: &str,
    tool: &str,
    title: &str,
    target: &str,
    reason: Option<String>,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<bool, String> {
    let Some(state) = app.try_state::<PendingToolApprovals>() else {
        return Ok(false);
    };

    let request_id = Uuid::new_v4().to_string();
    let rx = state.register(request_id.clone()).await;

    let _ = app.emit(
        "aider:tool_approval_request",
        ToolApprovalRequestEvent {
            request_id: request_id.clone(),
            conversation_id: conversation_id.to_string(),
            tool: tool.to_string(),
            title: title.to_string(),
            target: target.to_string(),
            reason,
        },
    );

    let result = state
        .await_decision(&request_id, rx, cancel_rx, Duration::from_secs(300))
        .await;

    match &result {
        Err(_) | Ok(false) => {
            let _ = app.emit("aider:tool_approval_resolved", (&request_id, false));
        }
        Ok(true) => {}
    }

    result
}

pub async fn resolve_and_emit(
    approvals: &PendingToolApprovals,
    app: &AppHandle,
    request_id: &str,
    approved: bool,
) -> bool {
    let resolved = approvals.resolve(request_id, approved).await;
    let _ = app.emit("aider:tool_approval_resolved", (request_id, approved));
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_approval_approve_and_deny() {
        let store = Arc::new(PendingToolApprovals::default());
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let rx1 = store.register("req-1".to_string()).await;
        assert!(store.resolve("req-1", true).await);
        let res1 = store
            .await_decision("req-1", rx1, &mut cancel_rx, Duration::from_secs(2))
            .await;
        assert_eq!(res1, Ok(true));

        let rx2 = store.register("req-2".to_string()).await;
        assert!(store.resolve("req-2", false).await);
        let res2 = store
            .await_decision("req-2", rx2, &mut cancel_rx, Duration::from_secs(2))
            .await;
        assert_eq!(res2, Ok(false));

        // Second resolve on same id returns false
        assert!(!store.resolve("req-2", true).await);
    }

    #[tokio::test]
    async fn test_approval_cancel_via_watch() {
        let store = Arc::new(PendingToolApprovals::default());
        let (cancel_tx, mut cancel_rx) = watch::channel(false);

        let rx = store.register("req-cancel".to_string()).await;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(15)).await;
            let _ = cancel_tx.send(true);
        });

        let res = store
            .await_decision("req-cancel", rx, &mut cancel_rx, Duration::from_secs(5))
            .await;
        assert_eq!(res, Err("CANCELLED_BY_USER".to_string()));
        assert!(!store.resolve("req-cancel", true).await);
    }

    #[tokio::test]
    async fn test_approval_timeout_fails_closed() {
        let store = Arc::new(PendingToolApprovals::default());
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let rx = store.register("req-timeout".to_string()).await;
        let res = store
            .await_decision("req-timeout", rx, &mut cancel_rx, Duration::from_millis(30))
            .await;
        assert_eq!(res, Ok(false));
        assert!(!store.resolve("req-timeout", true).await);
    }
}
