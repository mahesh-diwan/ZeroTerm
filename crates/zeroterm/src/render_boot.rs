//! Renderer boot supervisor. `Renderer::new` can stall for minutes
//! (adapter/device creation blocking on a loaded Intel iGPU, blocked on a
//! kernel futex with zero CPU) or panic (wgpu treats validation errors as
//! fatal). A stuck attempt must not keep the window dark forever, so after a
//! 10s timeout a FRESH attempt starts with its own Instance — a new driver
//! round-trip usually completes even when the previous one deadlocked.
//! First success wins; late successes pile up in the render channel and are
//! dropped. Abandoned stuck threads are leaked in that pathological case
//! only, and a device that never presents is inert.
//!
//! The retry policy is a pure function (`retry_policy`) so the timeout /
//! attempt-cap / give-up decisions are unit-testable without threads.

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use tracing::{error, warn};
use winit::window::Window;

use zeroterm_render::Renderer;

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const GIVE_UP_AFTER: Duration = Duration::from_secs(90);
const MAX_ATTEMPTS: u32 = 9;

/// What the supervisor should do next, decided purely by `retry_policy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootStep {
    Ready,
    GiveUp,
    Retry,
}

/// Pure retry policy: given the attempts fired so far, the give-up deadline,
/// the current time, and whether the last attempt finished OK (None = it
/// timed out without finishing), decide the next step. A timed-out attempt
/// counts as failed for the retry decision (the policy never sees a stuck
/// attempt as progress).
pub(crate) fn retry_policy(
    attempts: u32,
    max_attempts: u32,
    give_up_at: Instant,
    now: Instant,
    last_ok: Option<bool>,
) -> BootStep {
    match last_ok {
        Some(true) => BootStep::Ready,
        Some(false) | None => {
            if attempts >= max_attempts || now >= give_up_at {
                BootStep::GiveUp
            } else {
                BootStep::Retry
            }
        }
    }
}

/// Launch the renderer on a background supervisor thread and return the
/// channel the GUI polls for the first successful `Renderer`.
pub(crate) fn spawn_renderer(
    window: Arc<Window>,
    font_size: f32,
    opacity: f64,
    font_path: Option<String>,
) -> mpsc::Receiver<Renderer> {
    let (render_tx, render_rx) = mpsc::channel();
    std::thread::spawn(move || {
        crate::zt("renderer supervisor start");
        let (done_tx, done_rx) = mpsc::channel::<bool>();
        let mut attempts = 1u32;
        let give_up_at = Instant::now() + GIVE_UP_AFTER;

        // One attempt on its own thread: Renderer::new may block or panic; a
        // fresh thread isolates the supervisor loop from both.
        fn spawn_attempt(
            window: Arc<Window>,
            font_size: f32,
            opacity: f64,
            font_path: Option<String>,
            render_tx: mpsc::Sender<Renderer>,
            done_tx: mpsc::Sender<bool>,
        ) {
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    pollster::block_on(Renderer::new(window, font_size, opacity, font_path))
                }));
                match result {
                    Ok(Ok(r)) => {
                        let _ = render_tx.send(r);
                        let _ = done_tx.send(true);
                    }
                    Ok(Err(e)) => {
                        error!("Renderer init failed: {}", e);
                        let _ = done_tx.send(false);
                    }
                    Err(p) => {
                        let msg = p
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| p.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic".into());
                        error!("Renderer init panicked: {}", msg);
                        let _ = done_tx.send(false);
                    }
                }
            });
        }

        spawn_attempt(
            window.clone(),
            font_size,
            opacity,
            font_path.clone(),
            render_tx.clone(),
            done_tx.clone(),
        );
        loop {
            let last_ok = match done_rx.recv_timeout(ATTEMPT_TIMEOUT) {
                Ok(true) => Some(true),
                Ok(false) => Some(false),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("Renderer init supervisor channel closed; giving up");
                    break;
                }
            };
            match retry_policy(attempts, MAX_ATTEMPTS, give_up_at, Instant::now(), last_ok) {
                BootStep::Ready => break,
                BootStep::GiveUp => {
                    error!(
                        "Renderer init gave up after {} attempts; window stays dark",
                        attempts
                    );
                    window.set_title("ZeroTerm — GPU init failed (restart)");
                    break;
                }
                BootStep::Retry => {
                    attempts += 1;
                    warn!(
                        "Renderer init attempt {}: previous attempt not done in 10s, \
                         starting a fresh one",
                        attempts
                    );
                    spawn_attempt(
                        window.clone(),
                        font_size,
                        opacity,
                        font_path.clone(),
                        render_tx.clone(),
                        done_tx.clone(),
                    );
                }
            }
        }
        crate::zt("renderer supervisor done");
    });
    render_rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(secs: u64) -> Instant {
        Instant::now() + Duration::from_secs(secs)
    }

    #[test]
    fn success_is_ready() {
        assert_eq!(
            retry_policy(1, MAX_ATTEMPTS, instant(90), Instant::now(), Some(true)),
            BootStep::Ready
        );
    }

    #[test]
    fn failure_before_deadline_retries() {
        assert_eq!(
            retry_policy(1, MAX_ATTEMPTS, instant(90), Instant::now(), Some(false)),
            BootStep::Retry
        );
        // A timeout (None) counts like a failure: still retry while under cap.
        assert_eq!(
            retry_policy(2, MAX_ATTEMPTS, instant(90), Instant::now(), None),
            BootStep::Retry
        );
    }

    #[test]
    fn failure_at_max_attempts_gives_up() {
        assert_eq!(
            retry_policy(
                MAX_ATTEMPTS,
                MAX_ATTEMPTS,
                instant(90),
                Instant::now(),
                Some(false)
            ),
            BootStep::GiveUp
        );
        // Timed-out attempt at the cap must give up too, not spin forever.
        assert_eq!(
            retry_policy(
                MAX_ATTEMPTS,
                MAX_ATTEMPTS,
                instant(90),
                Instant::now(),
                None
            ),
            BootStep::GiveUp
        );
    }

    #[test]
    fn failure_after_deadline_gives_up() {
        let past = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            retry_policy(1, MAX_ATTEMPTS, past, Instant::now(), Some(false)),
            BootStep::GiveUp
        );
    }

    #[test]
    fn zero_attempts_retries_while_under_cap() {
        // The supervisor always starts at attempts=1, but the policy is
        // total: 0 attempts with a live deadline must Retry (nothing has been
        // exhausted yet), never spin into GiveUp prematurely.
        assert_eq!(
            retry_policy(0, MAX_ATTEMPTS, instant(90), Instant::now(), None),
            BootStep::Retry
        );
    }
}
