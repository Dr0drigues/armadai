//! The terminal side of `flow status --watch`: its size, the hidden cursor and
//! the signal that restores it. The only module of the binary with `unsafe`, tests aside:
//! each block says the invariant it relies on.

use crate::flow_status::watch::Size;

#[cfg(unix)]
const HIDE: &str = "\x1b[?25l";
#[cfg(unix)]
const SHOW: &str = "\x1b[?25h";

/// The terminal size: `COLUMNS`/`LINES`, else the size `ioctl(TIOCGWINSZ)` gives
/// for stdout, else 80x24, axis by axis.
pub fn size() -> Size {
    let ws = ioctl_size();
    let env = |k: &str| std::env::var(k).ok();
    Size {
        cols: pick(env("COLUMNS").as_deref(), ws.map(|w| w.0), 80),
        rows: pick(env("LINES").as_deref(), ws.map(|w| w.1), 24),
    }
}

/// One axis: the env value, else the ioctl value, else `default`; "0", empty or
/// not a number is skipped.
fn pick(env: Option<&str>, ioctl: Option<u16>, default: usize) -> usize {
    env.and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .or(ioctl.filter(|&n| n > 0).map(usize::from))
        .unwrap_or(default)
}

/// `(cols, rows)` of stdout, `None` when it is no terminal or says 0.
#[cfg(unix)]
fn ioctl_size() -> Option<(u16, u16)> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `ws` is a live, writable `winsize`, the only thing TIOCGWINSZ writes
    // through the pointer; a bad fd only makes `ioctl` return -1, checked below.
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    (rc == 0).then_some((ws.ws_col, ws.ws_row))
}

#[cfg(not(unix))]
fn ioctl_size() -> Option<(u16, u16)> {
    None
}

/// Hides the cursor while alive. On unix it also catches SIGINT to show the
/// cursor before dying of the signal, unless the process was started with SIGINT
/// ignored (or set to a handler): then it is left as it is. Elsewhere it does
/// nothing (the animation runs with the cursor visible).
pub struct HiddenCursor {
    /// Whether `on_sigint` was put in place, so `Drop` takes it out.
    #[cfg(unix)]
    caught: bool,
}

impl HiddenCursor {
    #[cfg(unix)]
    pub fn new() -> std::io::Result<Self> {
        use std::io::Write;
        // SAFETY: `old` is a zeroed `sigaction`, a valid value for the C struct, which
        // `sigaction` fills in (the null new action leaves the disposition unchanged).
        // `on_sigint` is an `extern "C" fn(c_int)`, the handler type `signal` expects,
        // and it only calls async-signal-safe functions. Reading before setting leaves
        // no window in which a SIGINT meant to be ignored would kill the process.
        let caught = unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            let read = libc::sigaction(libc::SIGINT, std::ptr::null(), &mut old);
            let default = read == 0 && old.sa_sigaction == libc::SIG_DFL;
            if default {
                libc::signal(
                    libc::SIGINT,
                    on_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
                );
            }
            default
        };
        // the handler is in place before the cursor goes: a Ctrl-C from here on shows it again;
        // `me` is dropped (cursor shown, SIG_DFL back if it was ours) if the write fails
        let me = HiddenCursor { caught };
        let mut out = std::io::stdout().lock();
        out.write_all(HIDE.as_bytes())?;
        out.flush()?;
        Ok(me)
    }

    #[cfg(not(unix))]
    pub fn new() -> std::io::Result<Self> {
        Ok(HiddenCursor {})
    }
}

#[cfg(unix)]
impl Drop for HiddenCursor {
    fn drop(&mut self) {
        use std::io::Write;
        // the cursor first: a Ctrl-C between the two lines still finds the handler
        {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(SHOW.as_bytes());
            let _ = out.flush();
        }
        if self.caught {
            // SAFETY: SIG_DFL is a valid disposition for SIGINT, and the one it had before.
            unsafe {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
            }
        }
    }
}

/// Ctrl-C: show the cursor, restore the default action and raise the signal
/// again, so the process dies of SIGINT as it did before the cursor was hidden.
/// Nothing but async-signal-safe calls (`write`, `signal`, `raise`).
#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int) {
    // SAFETY: `write`, `signal` and `raise` are async-signal-safe; the buffer is a
    // `'static` byte string and its length is its own.
    unsafe {
        libc::write(libc::STDOUT_FILENO, SHOW.as_ptr().cast(), SHOW.len());
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::raise(libc::SIGINT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_is_env_then_ioctl_then_default() {
        assert_eq!(pick(Some("132"), Some(100), 80), 132);
        assert_eq!(pick(Some("0"), Some(100), 80), 100);
        assert_eq!(pick(Some("x"), None, 80), 80);
        assert_eq!(pick(None, Some(0), 24), 24);
        assert_eq!(pick(Some(""), Some(7), 24), 7);
    }
}
