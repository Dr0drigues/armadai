//! Advisory locks on an open file, exclusive for a writer and shared for a reader, held until
//! the file is closed. The only `unsafe` of the journal feature; they serialise the check of a
//! file's last byte and the append that follows, and keep a reader from a half-written line,
//! between processes (a process that ignores the lock is not guarded against).

/// Blocks until this process holds `flock` on `f`, exclusive or shared; the lock goes with
/// the open file description, so it is released when `f` (and any duplicate) is dropped.
#[cfg(unix)]
fn flock_blocking(f: &std::fs::File, op: libc::c_int) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: `flock` takes a file descriptor and an integer and touches no memory; the
        // descriptor is open for as long as `f` is borrowed.
        let rc = unsafe { libc::flock(f.as_raw_fd(), op) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

#[cfg(unix)]
pub(crate) fn lock_exclusive(f: &std::fs::File) -> std::io::Result<()> {
    flock_blocking(f, libc::LOCK_EX)
}

/// A reader's lock: it waits for a writer's exclusive lock, so it never sees a half-written line.
#[cfg(unix)]
pub(crate) fn lock_shared(f: &std::fs::File) -> std::io::Result<()> {
    flock_blocking(f, libc::LOCK_SH)
}

/// shortcut: no lock off Unix, a torn-line check may then race a concurrent append; revisit
/// when the journal is used on Windows.
#[cfg(not(unix))]
pub(crate) fn lock_exclusive(_f: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn lock_shared(_f: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}

/// Whether the file is locked by another open file description, so that an exclusive lock
/// cannot be taken now (`true`). When it is free this takes the lock, kept until `f` is
/// dropped, and returns `false`. For tests.
#[cfg(all(test, unix))]
pub(crate) fn is_locked_elsewhere(f: &std::fs::File) -> bool {
    use std::os::fd::AsRawFd;
    // SAFETY: as in `flock_blocking`; LOCK_NB makes the call return at once.
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    rc != 0
}
