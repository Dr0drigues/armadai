//! An exclusive advisory lock on an open file, held until the file is closed. The only
//! `unsafe` of the journal feature; it serialises the check of a file's last byte and the
//! append that follows, between processes (it does not guard against a process that ignores
//! the lock).

/// Blocks until this process holds `flock(LOCK_EX)` on `f`; the lock goes with the open file
/// description, so it is released when `f` (and any duplicate) is dropped.
#[cfg(unix)]
pub(crate) fn lock_exclusive(f: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: `flock` takes a file descriptor and an integer and touches no memory; the
        // descriptor is open for as long as `f` is borrowed.
        let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

/// shortcut: no lock off Unix, a torn-line check may then race a concurrent append; revisit
/// when the journal is used on Windows.
#[cfg(not(unix))]
pub(crate) fn lock_exclusive(_f: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
