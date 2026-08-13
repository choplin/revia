//! Physical terminal rendering isolated from the input/update loop.
//!
//! The gate accepts a semantic frame only while the physical renderer is idle.
//! Input can continue updating the model while a frame is in flight; the app
//! builds the next logical frame only after the renderer becomes ready again.

use std::{
    io,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{renderer::Renderer, semantic::View};

#[derive(Debug)]
struct MailboxState<T> {
    pending: Option<T>,
    drawing: bool,
    closed: bool,
}

#[derive(Debug)]
struct FrameGate<T> {
    state: Mutex<MailboxState<T>>,
    ready: Condvar,
}

impl<T> FrameGate<T> {
    fn new() -> Self {
        Self {
            state: Mutex::new(MailboxState {
                pending: None,
                drawing: false,
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    fn try_publish(&self, value: T) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed || state.drawing || state.pending.is_some() {
            return false;
        }
        state.pending = Some(value);
        self.ready.notify_one();
        true
    }

    fn is_ready(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        !state.closed && !state.drawing && state.pending.is_none()
    }

    fn receive(&self) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(value) = state.pending.take() {
                state.drawing = true;
                return Some(value);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn complete(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.drawing = false;
        self.ready.notify_all();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.pending = None;
        self.ready.notify_one();
    }
}

pub(crate) struct PhysicalRenderer {
    mailbox: Arc<FrameGate<View>>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl PhysicalRenderer {
    pub(crate) fn start() -> Result<Self> {
        let mailbox = Arc::new(FrameGate::new());
        let receiver = Arc::clone(&mailbox);
        let worker = thread::Builder::new()
            .name("revia-render".into())
            .spawn(move || render_frames(&receiver))
            .context("could not start terminal renderer")?;
        Ok(Self {
            mailbox,
            worker: Some(worker),
        })
    }

    pub(crate) fn try_publish(&self, view: View) -> bool {
        self.mailbox.try_publish(view)
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.mailbox.is_ready()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }

    pub(crate) fn finish(mut self) -> Result<()> {
        self.mailbox.close();
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("terminal renderer panicked"))?
    }
}

impl Drop for PhysicalRenderer {
    fn drop(&mut self) {
        self.mailbox.close();
        // TerminalSession is dropped after this value during unwinding. Wait
        // for any in-flight stdout write before the session restores the
        // alternate screen and raw mode.
        let _ = self.join_worker();
    }
}

fn render_frames(mailbox: &FrameGate<View>) -> Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("could not initialize terminal renderer")?;
    let renderer = Renderer::default();
    while let Some(view) = mailbox.receive() {
        terminal
            .draw(|frame| renderer.render(frame, &view))
            .context("could not draw terminal frame")?;
        mailbox.complete();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::FrameGate;

    #[test]
    fn in_flight_frame_rejects_another_logical_frame_until_completion() {
        let mailbox = FrameGate::new();
        assert!(mailbox.try_publish(1));
        assert_eq!(mailbox.receive(), Some(1));
        assert!(!mailbox.is_ready());
        assert!(!mailbox.try_publish(2));

        mailbox.complete();
        assert!(mailbox.is_ready());
        assert!(mailbox.try_publish(3));
        assert_eq!(mailbox.receive(), Some(3));
    }

    #[test]
    fn closing_discards_pending_work_and_stops_the_receiver() {
        let mailbox = FrameGate::new();
        assert!(mailbox.try_publish(1));
        mailbox.close();

        assert_eq!(mailbox.receive(), None);
    }
}
