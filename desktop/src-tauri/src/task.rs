//! The one long-running job the app does at a time (a start, stop or
//! build) and what its steps share: the stack, a way to report output, and
//! the process to stop on cancel.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::stack;

pub struct Running {
    pub kind: String,
    /// The process the current step is waiting on, if any.
    pub child: Option<Arc<Mutex<Child>>>,
    pub cancelled: bool,
}

impl Running {
    /// Marks the task cancelled and stops its current process.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        if let Some(child) = &self.child {
            stack::interrupt(&mut child.lock().unwrap());
        }
    }
}

pub type Slot = Arc<Mutex<Option<Running>>>;

#[derive(Clone)]
pub struct Ctx {
    pub handle: AppHandle,
    pub dir: PathBuf,
    pub slot: Slot,
}

impl Ctx {
    /// Adds a line to the Activity pane.
    pub fn line(&self, line: impl Into<String>) {
        let _ = self.handle.emit("task-line", line.into());
    }

    /// Records the process the task is waiting on, so a cancel stops it.
    pub fn track(&self, child: Option<Arc<Mutex<Child>>>) {
        if let Some(running) = self.slot.lock().unwrap().as_mut() {
            running.child = child;
        }
    }

    pub fn check_cancelled(&self) -> Result<(), String> {
        match self.slot.lock().unwrap().as_ref() {
            Some(r) if r.cancelled => Err("Cancelled.".into()),
            _ => Ok(()),
        }
    }

    /// Runs a command to completion, streaming its output to the Activity pane.
    pub fn run(&self, cmd: Command) -> Result<(), String> {
        let lines = self.clone();
        let (child, done) = stack::spawn_lines(cmd, move |line| lines.line(line)).map_err(|e| e.to_string())?;
        let child = Arc::new(Mutex::new(child));
        self.track(Some(child.clone()));
        let _ = done.join();
        // The streams are closed, so the process has ended or is about to.
        let status = wait(&child)?;
        self.track(None);
        if status.success() {
            return Ok(());
        }
        self.check_cancelled()?;
        Err(match status.code() {
            Some(code) => format!("The command stopped with exit code {code}."),
            None => "The command was stopped.".into(),
        })
    }
}

/// Waits for a tracked process without holding its lock, so a cancel can
/// still reach it.
pub fn wait(child: &Arc<Mutex<Child>>) -> Result<std::process::ExitStatus, String> {
    loop {
        if let Some(status) = child.lock().unwrap().try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
