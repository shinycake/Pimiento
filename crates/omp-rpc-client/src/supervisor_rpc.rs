//! Thin adapter from [`crate::client::RpcClient`] onto the supervisor
//! [`ProcessHandle`](crate::supervisor::ProcessHandle) surface.
//!
//! Kept separate from [`crate::supervisor`] so the policy state machine stays
//! free of concrete client / smol-process dependencies in its unit tests.

use smol::channel;

use crate::client::{ClosedInfo as ClientClosedInfo, RpcClient};
use crate::supervisor::{
    self, ClosedInfo, GRACEFUL_STDIN_CLOSE_DEADLINE, ProcessHandle, shutdown_child,
};

/// Map a client-layer [`ClientClosedInfo`] into the supervisor's
/// wire-compatible [`ClosedInfo`].
#[must_use]
pub fn map_closed(info: &ClientClosedInfo) -> ClosedInfo {
    ClosedInfo {
        exit_code: info.exit_code,
        stderr_tail: info.stderr_tail.clone(),
        error_msg: info.error_msg.clone(),
    }
}

/// [`ProcessHandle`] wrapper around a live [`RpcClient`].
#[derive(Debug)]
pub struct RpcClientHandle(pub RpcClient);

impl ProcessHandle for RpcClientHandle {
    fn close_stdin(&self) {
        let client = self.0.clone();
        smol::spawn(async move {
            client.close_stdin().await;
        })
        .detach();
    }

    fn kill(&self) {
        let client = self.0.clone();
        smol::spawn(async move {
            let _ = client.kill().await;
        })
        .detach();
    }

    fn terminated(&self) -> channel::Receiver<ClosedInfo> {
        let (tx, rx) = channel::bounded(1);
        let client = self.0.clone();
        smol::spawn(async move {
            let info = map_closed(&client.wait().await);
            let _ = tx.send(info).await;
        })
        .detach();
        rx
    }

    fn wait(self: Box<Self>) -> channel::Receiver<ClosedInfo> {
        let (tx, rx) = channel::bounded(1);
        let client = self.0;
        smol::spawn(async move {
            let info = map_closed(&client.wait().await);
            let _ = tx.send(info).await;
        })
        .detach();
        rx
    }
}

/// Drive the PLAN §4.8 graceful shutdown sequence against an [`RpcClient`]:
/// close stdin → wait up to [`GRACEFUL_STDIN_CLOSE_DEADLINE`] → kill → reap.
pub async fn gracefully_shutdown_rpc_client(client: RpcClient) -> supervisor::ClosedInfo {
    let handle: Box<dyn ProcessHandle> = Box::new(RpcClientHandle(client));
    let (_outcome, info) = shutdown_child(handle, GRACEFUL_STDIN_CLOSE_DEADLINE).await;
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_closed_copies_all_fields() {
        let info = ClientClosedInfo {
            exit_code: Some(42),
            stderr_tail: "tail".into(),
            error_msg: Some("boom".into()),
        };
        let mapped = map_closed(&info);
        assert_eq!(mapped.exit_code, Some(42));
        assert_eq!(mapped.stderr_tail, "tail");
        assert_eq!(mapped.error_msg.as_deref(), Some("boom"));
    }

    #[test]
    fn map_closed_preserves_none_fields() {
        let info = ClientClosedInfo {
            exit_code: None,
            stderr_tail: String::new(),
            error_msg: None,
        };
        let mapped = map_closed(&info);
        assert_eq!(mapped.exit_code, None);
        assert!(mapped.stderr_tail.is_empty());
        assert_eq!(mapped.error_msg, None);
    }
}
