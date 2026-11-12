//! Process-group helpers for a child we spawned with `process_group(0)`: seeing that its
//! leader has exited without reaping it, and killing its whole group. The only module
//! with `unsafe` for this feature; the invariant is that a group is signalled only while
//! its leader is alive or a zombie, so the pid still names the group we created.

use std::time::{Duration, Instant};

/// What the leader of a process group is doing, seen without reaping it.
#[derive(Debug, PartialEq)]
pub(crate) enum Leader {
    Running,
    /// A zombie: it keeps its pid, hence the id of its group, until it is waited for.
    Exited,
    /// `waitid` failed (ECHILD: it was reaped, or auto-reaped with SIGCHLD ignored): its pid
    /// may name anything now.
    Gone,
}

/// The state of `child`, which was spawned with `process_group(0)`, without reaping it.
#[cfg(unix)]
pub(crate) fn leader(child: &mut std::process::Child) -> Leader {
    // SAFETY: `info` is a zeroed `siginfo_t`, a valid value for the C struct, and the only
    // thing `waitid` writes through the pointer; `child.id()` is a pid we spawned, and
    // WNOHANG | WNOWAIT make the call neither block nor reap. With nothing to report `info`
    // stays zeroed; a leader that has exited sets `si_signo` to SIGCHLD.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let rc = libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        match (rc, info.si_signo == libc::SIGCHLD) {
            (0, true) => Leader::Exited,
            (0, false) => Leader::Running,
            _ => Leader::Gone,
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn leader(child: &mut std::process::Child) -> Leader {
    match child.try_wait() {
        Ok(None) => Leader::Running,
        Ok(Some(_)) => Leader::Exited,
        Err(_) => Leader::Gone,
    }
}

/// SIGKILL to every process of the group `child` leads. Only for a leader seen `Running` or
/// `Exited` by [`leader`] and not waited for since.
#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    // SAFETY: `kill` takes two integers and touches no memory. The leader was seen alive or
    // as a zombie and has not been waited for, so its pid is still the id of the group we
    // created and no process of anyone else can carry it; a group already empty makes
    // `kill` return -1 (ESRCH), which we ignore. One window stays, on Linux with SIGCHLD
    // ignored (the kernel reaps at once): a leader seen `Running` may exit and be reaped
    // between `leader` and this call, a few microseconds, and its pid could in theory be
    // reused by then; the odds are negligible, and nothing narrows them further.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// How [`finish`] ended.
#[derive(Debug)]
pub(crate) enum Finish {
    Exited(std::process::ExitStatus),
    TimedOut,
    /// The leader could not be seen or waited for: no group was signalled.
    Lost,
}

/// Wait for `child` (spawned with `process_group(0)`) for at most `limit`. At the limit its
/// whole group is killed; after a normal end only when `kill_after_exit`. A group is never
/// signalled unless the leader was seen alive or exited and not reaped.
pub(crate) fn finish(
    child: &mut std::process::Child,
    limit: Duration,
    kill_after_exit: bool,
) -> Finish {
    let start = Instant::now();
    loop {
        match leader(child) {
            Leader::Exited => {
                if kill_after_exit {
                    kill_group(child);
                }
                return child.wait().map_or(Finish::Lost, Finish::Exited);
            }
            Leader::Gone => return Finish::Lost,
            Leader::Running if start.elapsed() >= limit => {
                kill_group(child);
                let _ = child.wait();
                return Finish::TimedOut;
            }
            Leader::Running => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn alive(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// `kill -9 <pid>` when dropped, so a failing assertion leaves no process behind.
    struct KillOnDrop(String);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = Command::new("kill").args(["-9", &self.0]).status();
        }
    }

    /// `kill -0` also succeeds on a zombie until init reaps it: allow 2 s.
    fn pid_alive_for_2s(pid: &str) -> bool {
        (0..200).all(|_| {
            let a = alive(pid);
            if a {
                std::thread::sleep(Duration::from_millis(10));
            }
            a
        })
    }

    #[cfg(unix)]
    #[test]
    fn leader_does_not_reap_the_leader_so_its_group_id_stays_ours() {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new("sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .spawn()
            .unwrap();
        let start = Instant::now();
        while leader(&mut child) != Leader::Exited {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        // a reaped leader would be ESRCH here: a zombie still holds the pid and the group id
        assert!(alive(&child.id().to_string()), "the leader was reaped");
        kill_group(&mut child);
        assert!(child.wait().unwrap().success());
    }

    /// The leader is reaped while a member of its group lives on: `waitid` then fails, as
    /// it does with SIGCHLD ignored on Linux (not reachable on macOS), and nothing may be
    /// signalled, since the pid no longer names our group.
    #[cfg(unix)]
    #[test]
    fn a_leader_that_cannot_be_seen_gets_no_group_kill() {
        use std::os::unix::process::CommandExt;
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("pid");
        let command = format!("sleep 30 >/dev/null 2>&1 & echo $! > {}", file.display());
        let mut child = Command::new("sh")
            .args(["-c", &command])
            .process_group(0)
            .spawn()
            .unwrap();
        child.wait().unwrap();
        // the guard kills the `sleep` whatever an assertion below says
        let _cleanup = KillOnDrop(std::fs::read_to_string(&file).unwrap().trim().to_string());
        let pid = &_cleanup.0;
        assert_eq!(leader(&mut child), Leader::Gone);
        let outcome = finish(&mut child, Duration::from_secs(1), true);
        let still = pid_alive_for_2s(pid);
        assert!(matches!(outcome, Finish::Lost), "{outcome:?}");
        assert!(still, "the group of a leader not seen was killed");
    }
}
