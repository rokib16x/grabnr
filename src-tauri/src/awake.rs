//! Keep the computer from going to sleep on its own while downloads run.
//!
//! Idle sleep cuts the network: every connection dies, torrent peers are dropped and pile up long reconnect delays,
//! and a download left running overnight is found stalled in the morning. Only idle sleep is held off; closing the lid
//! or choosing Sleep still sleeps, and the hold is released as soon as nothing is downloading.

use std::sync::Mutex;

pub struct KeepAwake(Mutex<Option<Hold>>);

impl KeepAwake {
    pub fn new() -> Self {
        KeepAwake(Mutex::new(None))
    }

    /// Hold off idle sleep while `on`, release it otherwise. Cheap to call repeatedly.
    pub fn set(&self, on: bool) {
        let mut g = self.0.lock().unwrap();
        match (on, g.is_some()) {
            (true, false) => *g = Hold::start(),
            (false, true) => *g = None, // dropping the hold releases it
            _ => {}
        }
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

/// macOS and Linux: a helper process holds the assertion and is told to exit with grabnr, so a crash cannot leave
/// the computer unable to sleep.
#[cfg(not(windows))]
struct Hold(std::process::Child);

#[cfg(not(windows))]
impl Hold {
    fn start() -> Option<Self> {
        let pid = std::process::id().to_string();
        #[cfg(target_os = "macos")]
        let mut cmd = {
            // -i: no idle system sleep. -w: give up when grabnr exits.
            let mut c = std::process::Command::new("/usr/bin/caffeinate");
            c.args(["-i", "-w", &pid]);
            c
        };
        #[cfg(not(target_os = "macos"))]
        let mut cmd = {
            let mut c = std::process::Command::new("systemd-inhibit");
            c.args([
                "--what=idle:sleep",
                "--who=grabnr",
                "--why=Downloading",
                "--mode=block",
                "tail",
                &format!("--pid={pid}"),
                "-f",
                "/dev/null",
            ]);
            c
        };
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().ok().map(Hold)
    }
}

#[cfg(not(windows))]
impl Drop for Hold {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Windows: the request belongs to the thread that makes it, so a small thread keeps it until told to let go.
#[cfg(windows)]
struct Hold(Option<std::sync::mpsc::Sender<()>>);

#[cfg(windows)]
impl Hold {
    fn start() -> Option<Self> {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                let _ = rx.recv(); // returns when the hold is dropped
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            })
            .ok()?;
        Some(Hold(Some(tx)))
    }
}

#[cfg(windows)]
impl Drop for Hold {
    fn drop(&mut self) {
        drop(self.0.take()); // closes the channel, so the thread clears the request and ends
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn assertion_held() -> bool {
        let out = std::process::Command::new("pmset").args(["-g", "assertions"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.contains("caffeinate command-line tool") && l.contains("PreventUserIdleSystemSleep"))
    }

    #[test]
    fn holds_and_releases_idle_sleep() {
        let k = KeepAwake::new();
        k.set(true);
        k.set(true); // no second helper
        std::thread::sleep(std::time::Duration::from_millis(300));
        let pid = k.0.lock().unwrap().as_ref().map(|h| h.0.id()).expect("a helper runs");
        assert!(assertion_held(), "caffeinate holds the assertion");
        k.set(false);
        assert!(k.0.lock().unwrap().is_none());
        let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap().success();
        assert!(!alive, "the helper is gone once released");
    }
}
