//! Runs closures on the engine (main) thread on behalf of MCP request threads.
//!
//! GPU, window and scene state live on the main thread. A tool handler sends a [`Job`] here,
//! wakes the event loop, and blocks until the job's result comes back (or times out).

use crate::engine::Engine;
use oxide_mcp::ToolResult;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

pub type Job = Box<dyn FnOnce(&mut Engine) -> ToolResult + Send>;
pub type Envelope = (Job, SyncSender<ToolResult>);

/// Event sent to wake the loop when a job is queued.
#[derive(Debug, Clone, Copy)]
pub struct Wake;

const TIMEOUT: Duration = Duration::from_secs(15);

pub struct Bus {
    tx: Mutex<Sender<Envelope>>,
    proxy: EventLoopProxy<Wake>,
}

impl Bus {
    pub fn new(proxy: EventLoopProxy<Wake>) -> (Self, Receiver<Envelope>) {
        let (tx, rx) = channel();
        (
            Self {
                tx: Mutex::new(tx),
                proxy,
            },
            rx,
        )
    }

    /// Runs `job` on the engine thread and waits for its result.
    pub fn run(&self, job: impl FnOnce(&mut Engine) -> ToolResult + Send + 'static) -> ToolResult {
        let (reply_tx, reply_rx) = sync_channel(1);
        let sent = self
            .tx
            .lock()
            .map(|tx| tx.send((Box::new(job), reply_tx)).is_ok())
            .unwrap_or(false);
        if !sent || self.proxy.send_event(Wake).is_err() {
            return ToolResult::error("the engine is shutting down");
        }
        reply_rx.recv_timeout(TIMEOUT).unwrap_or_else(|_| {
            ToolResult::error(format!(
                "the engine did not respond within {}s (still starting, or the main thread is blocked)",
                TIMEOUT.as_secs()
            ))
        })
    }
}
