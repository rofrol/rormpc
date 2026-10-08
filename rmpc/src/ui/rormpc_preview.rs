//! rormpc: the preview player of the Versions pane and the Hits pane's Downloads review. A file plays in a separate
//! player (mpv, else ffplay), never through MPD, whose scrobbler would log a listen or a skip. One preview per
//! pane: starting another replaces it, and the pane stops it when it hides or goes away.

use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::Mutex,
};

use crate::shared::macros::status_error;

/// The preview player: its process and what it plays.
#[derive(Debug, Default)]
pub struct Preview {
    child: Option<Child>,
    pub what: String,
}

impl Preview {
    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.what.clear();
    }

    /// Still playing? Clears itself when the player has ended.
    pub fn playing(&mut self) -> bool {
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                self.what.clear();
                false
            }
            None => false,
        }
    }
}

/// The player command for a preview: mpv, else ffplay; none if neither is installed.
fn player_command(path: &Path, start: u32) -> Option<Command> {
    if which::which("mpv").is_ok() {
        let mut c = Command::new("mpv");
        c.args(["--no-video", "--really-quiet", &format!("--start={start}")]).arg(path);
        return Some(c);
    }
    if which::which("ffplay").is_ok() {
        let mut c = Command::new("ffplay");
        c.args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-ss", &start.to_string()]).arg(path);
        return Some(c);
    }
    None
}

/// Play `path` from `start` seconds, replacing a running preview; `what` is shown while it plays.
pub fn start(preview: &Mutex<Preview>, path: &Path, start: u32, what: String) {
    let mut p = preview.lock().expect("preview lock");
    p.stop();
    let Some(mut cmd) = player_command(path, start) else {
        status_error!("preview needs mpv or ffplay");
        return;
    };
    match cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => {
            p.child = Some(child);
            p.what = what;
        }
        Err(err) => status_error!("preview: {err}"),
    }
}
