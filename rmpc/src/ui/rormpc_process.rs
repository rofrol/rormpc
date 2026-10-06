//! Local mpd-player liveness. MPD's `UnsubscribeAll` on disconnect does NOT
//! emit Subscription (0.24.15), so a cached channels response cannot detect a
//! crash. One native process-exit wait per daemon session; no polling, sleep or
//! extra freshness timeout. Failure to establish observation is deliberately
//! stale.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        Arc,
        Mutex,
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use crossbeam::channel::Sender;

use crate::shared::events::AppEvent;

struct ProcessWatch {
    pid: u32,
    session: String,
    alive: Arc<AtomicBool>,
}
static WATCH: OnceLock<Mutex<Option<ProcessWatch>>> = OnceLock::new();

pub fn alive(pid: u32, version: &str, tx: &Sender<AppEvent>) -> bool {
    let Some((session, _)) = version.split_once(':') else { return false };
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    let Ok(mut slot) = WATCH.get_or_init(|| Mutex::new(None)).lock() else { return false };
    if !slot.as_ref().is_some_and(|w| w.pid == pid && w.session == session) {
        *slot =
            Some(ProcessWatch { pid, session: session.to_owned(), alive: start(pid, tx.clone()) });
    }
    slot.as_ref().is_some_and(|w| w.alive.load(Ordering::Relaxed))
}

fn start(pid: u32, tx: Sender<AppEvent>) -> Arc<AtomicBool> {
    let alive = Arc::new(AtomicBool::new(false));
    let fd = match open(pid) {
        Ok(fd) => fd,
        Err(err) => {
            log::debug!(pid, err:?; "Cannot observe mpd-player; forecast is stale");
            return alive;
        }
    };
    alive.store(true, Ordering::Relaxed);
    let flag = Arc::clone(&alive);
    if let Err(err) = std::thread::Builder::new().name("mpd-player-exit".into()).spawn(move || {
        if let Err(err) = wait(&fd) {
            log::debug!(err:?; "Process-exit observation failed; forecast is stale");
        }
        flag.store(false, Ordering::Relaxed);
        let _ = tx.send(AppEvent::RequestRender);
    }) {
        log::warn!(err:?; "Cannot start mpd-player exit observer");
        alive.store(false, Ordering::Relaxed);
    }
    alive
}

#[cfg(target_os = "macos")]
fn open(pid: u32) -> io::Result<OwnedFd> {
    // SAFETY: kqueue takes no pointers and yields a new descriptor exclusively
    // owned below.
    let fd = unsafe { libc::kqueue() };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a newly created valid descriptor, not owned anywhere else.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let event = libc::kevent {
        ident: pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: one initialized change, no output array, no timeout. Registration
    // is atomic with exit observation.
    if unsafe {
        libc::kevent(fd.as_raw_fd(), &raw const event, 1, std::ptr::null_mut(), 0, std::ptr::null())
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

#[cfg(target_os = "macos")]
fn wait(fd: &OwnedFd) -> io::Result<()> {
    let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
    loop {
        // SAFETY: valid owned descriptor, no changes, storage for one event, no
        // timeout: wait for process exit.
        let n = unsafe {
            libc::kevent(
                fd.as_raw_fd(),
                std::ptr::null(),
                0,
                event.as_mut_ptr(),
                1,
                std::ptr::null(),
            )
        };
        if n > 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        } // resume an interrupted OS wait, not a retry of a command
    }
}

#[cfg(target_os = "linux")]
fn open(pid: u32) -> io::Result<OwnedFd> {
    // SAFETY: pidfd_open takes only a validated positive pid and flags 0,
    // yielding a new owned descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a newly created valid descriptor, not owned elsewhere.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as std::os::fd::RawFd) })
}

#[cfg(target_os = "linux")]
fn wait(fd: &OwnedFd) -> io::Result<()> {
    let mut event = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    loop {
        // SAFETY: initialized storage for one pollfd and its owned descriptor;
        // -1 means no timeout, no polling interval.
        let n = unsafe { libc::poll(&mut event, 1, -1) };
        if n > 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use libc as _;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open(_: u32) -> io::Result<OwnedFd> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "No native process-exit observer"))
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn wait(_: &OwnedFd) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "No native process-exit observer"))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };

    use super::*;

    #[test]
    fn process_exit_requests_a_render_without_a_timer() {
        let mut child =
            Command::new("sh").args(["-c", "read line"]).stdin(Stdio::piped()).spawn().unwrap();
        let (tx, rx) = crossbeam::channel::unbounded();
        let alive = start(child.id(), tx);
        assert!(alive.load(Ordering::Relaxed));
        drop(child.stdin.take()); // EOF is the observable condition which makes our scratch child exit
        child.wait().unwrap();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(2)), Ok(AppEvent::RequestRender))); // external OS notification deadline
        assert!(!alive.load(Ordering::Relaxed));
    }

    #[test]
    fn already_dead_process_is_not_live() {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let (tx, _) = crossbeam::channel::unbounded();
        assert!(!super::alive(pid, "dead:1", &tx));
    }
}
