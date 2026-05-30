//! Fallback for platforms without a supported job mechanism (e.g. macOS, BSD):
//! spawn directly, with no containment. Documented as best-effort — the parent
//! exiting does *not* reap descendants here.

use std::io;
use std::process::Command;

use crate::Mechanism;

pub struct Job;

impl Job {
    pub fn new() -> io::Result<Self> {
        Ok(Job)
    }

    pub fn spawn(&self, cmd: &mut Command) -> io::Result<std::process::Child> {
        cmd.spawn()
    }

    pub fn kill_all(&self) -> io::Result<()> {
        Ok(())
    }

    pub fn mechanism(&self) -> Mechanism {
        Mechanism::None
    }
}
