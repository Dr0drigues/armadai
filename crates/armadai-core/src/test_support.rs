//! Shared test-only environment isolation.
//!
//! Tests across this workspace mutate process-global state — the
//! `ARMADAI_CONFIG_DIR` env var, `HOME`, the current directory — and so must
//! be serialised against each other. They all serialise on the single
//! [`ENV_MUTEX`], but they used to each acquire it, and each re-implement the
//! save/restore guard around it, on their own. That duplication had already
//! diverged: one copy tolerated poisoning while every other copy did not (see
//! [`env_lock`]), and two ~50-line `ProjectDirGuard` copies differed only by a
//! doc sentence.
//!
//! Everything here is test-only: gated behind `cfg(test)` for this crate and
//! the `test-support` feature for downstream crates (enabled from their
//! `[dev-dependencies]`, so a release build never pulls it in).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Global mutex serialising every test that mutates process-global
/// environment state.
///
/// Private on purpose: acquire it through [`env_lock`], never directly. A
/// direct `ENV_MUTEX.lock().unwrap()` is exactly the defect issue #365
/// measured, and making the static unreachable is what stops it coming back.
static ENV_MUTEX: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

/// Acquire `mutex`, recovering the guard instead of panicking when a previous
/// holder panicked while holding it.
///
/// Split out from [`env_lock`] so the poison-tolerance itself can be tested
/// against a throwaway mutex, without poisoning the process-wide [`ENV_MUTEX`]
/// that every other test in the run depends on.
fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Acquire the shared environment lock, tolerating poisoning.
///
/// Poison-tolerant because [`ENV_MUTEX`] guards `()`: there is no shared data
/// a panicking holder could have left inconsistent, only the env-var/cwd
/// restoration each guard's own `Drop` already performs — and `Drop` runs
/// during unwinding, before the poison is even set.
///
/// That reasoning holds for the RAII guards below. It does **not** hold for
/// the ~19 call sites that take this lock and then restore by hand, with no
/// `Drop`: there, tolerating poison trades a cascade of phantom
/// `PoisonError`s for one leaked env var propagating quietly into whichever
/// tests run next. Measured (#372 review): 3 downstream tests read a value
/// leaked by a deliberately failed `skill::tests` case — against 5 before
/// this change, so an improvement rather than a regression, and it cannot
/// turn CI falsely green (a test must already be failing for anything to
/// leak at all). The real fix for those sites is `IsolatedConfigDir`, which
/// does exactly what they open-code, safely.
///
/// Measured (issue #365, `--test-threads=1`, `--bin armadai`): with
/// `.unwrap()` here, one deliberately failing test holding this lock turned
/// into a cascade of phantom `PoisonError`s across unrelated modules, burying
/// the one real failure — 74 with the red test early in the run, 8 with it
/// late, since the blast radius is a function of how many sites still take
/// the lock afterwards. With the tolerance, the same mutation gives
/// `693 passed; 1 failed` and zero phantoms.
pub fn env_lock() -> MutexGuard<'static, ()> {
    lock_ignoring_poison(&ENV_MUTEX)
}

/// Points `ARMADAI_CONFIG_DIR` at a fresh, empty temp dir for the guard's
/// lifetime, restoring the previous value (present or absent) on drop — while
/// holding [`env_lock`] throughout.
///
/// Without it, anything reading the user config (`AppPaths::resolve`, the
/// global agent library `load_all_agents` scans for shadowing collisions, …)
/// reads whatever `~/.config/armadai/` happens to hold on the machine running
/// the test.
pub struct IsolatedConfigDir {
    _lock: MutexGuard<'static, ()>,
    orig_config_dir: Option<String>,
    /// Extra variables set through [`IsolatedConfigDir::with_var`], in the
    /// order they were set, each paired with the value it displaced.
    saved_vars: Vec<(String, Option<String>)>,
    config_tmp: tempfile::TempDir,
}

impl IsolatedConfigDir {
    pub fn enter() -> Self {
        let lock = env_lock();
        let orig_config_dir = std::env::var("ARMADAI_CONFIG_DIR").ok();
        let config_tmp = tempfile::tempdir().expect("temp config dir");
        // SAFETY: serialised via the lock held above.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", config_tmp.path());
        }
        Self {
            _lock: lock,
            orig_config_dir,
            saved_vars: Vec::new(),
            config_tmp,
        }
    }

    /// Set (`Some`) or unset (`None`) one more environment variable for this
    /// guard's lifetime, restoring whatever it displaced on drop. Chainable.
    ///
    /// Code under test reads more than `ARMADAI_CONFIG_DIR` — a provider's
    /// `*_API_KEY` or `*_BASE_URL`, say — and a test that wants to pin those
    /// needs the same save/restore discipline, under the same
    /// [`env_lock`]. Without this the test writes its own guard, which is
    /// the duplication this module exists to end: `providers::factory`'s
    /// `EnvScope` (#368) was a ~50-line copy of exactly this, and it stopped
    /// compiling the moment #372 moved the lock.
    ///
    /// Setting the same name twice is fine: the values are restored in
    /// reverse order, so the original still wins.
    pub fn with_var(mut self, name: &str, value: Option<&str>) -> Self {
        self.saved_vars
            .push((name.to_string(), std::env::var(name).ok()));
        // SAFETY: serialised via the lock held by `self._lock`.
        unsafe {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        self
    }

    /// The isolated config dir itself — for a test that wants to plant a
    /// `config.yaml` in it (redirecting storage, say).
    pub fn config_dir(&self) -> &Path {
        self.config_tmp.path()
    }

    /// The isolated global agent library (`<ARMADAI_CONFIG_DIR>/agents`) — for
    /// a test that wants to plant a same-named `.md` there to trigger a
    /// declared/global shadowing collision deliberately.
    pub fn global_agents_dir(&self) -> PathBuf {
        self.config_tmp.path().join("agents")
    }
}

impl Drop for IsolatedConfigDir {
    fn drop(&mut self) {
        // SAFETY: restoring original env state, still under the lock held by
        // `self._lock` until this `Drop` returns.
        unsafe {
            // Reverse order, so a name set twice ends on its original value.
            for (name, value) in self.saved_vars.iter().rev() {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
            match &self.orig_config_dir {
                Some(v) => std::env::set_var("ARMADAI_CONFIG_DIR", v),
                None => std::env::remove_var("ARMADAI_CONFIG_DIR"),
            }
        }
    }
}

/// [`IsolatedConfigDir`] plus a process cwd moved to `project_root`, restored
/// on drop.
///
/// Composed rather than merged: plenty of tests need only the config-dir half,
/// and moving the cwd is the strictly stronger, strictly rarer need — a
/// surface resolving its project through the cwd-reading
/// `project::find_project_config()` (as opposed to its `_from(start)` twin,
/// which needs no guard at all).
pub struct IsolatedProjectDir {
    config: IsolatedConfigDir,
    orig_cwd: PathBuf,
}

impl IsolatedProjectDir {
    pub fn enter(project_root: &Path) -> Self {
        let config = IsolatedConfigDir::enter();
        let orig_cwd = std::env::current_dir().expect("readable cwd");
        std::env::set_current_dir(project_root).expect("cwd to project root");
        Self { config, orig_cwd }
    }

    /// See [`IsolatedConfigDir::config_dir`].
    pub fn config_dir(&self) -> &Path {
        self.config.config_dir()
    }

    /// See [`IsolatedConfigDir::global_agents_dir`].
    pub fn global_agents_dir(&self) -> PathBuf {
        self.config.global_agents_dir()
    }
}

impl Drop for IsolatedProjectDir {
    fn drop(&mut self) {
        // Runs before `self.config` drops, so the cwd is restored while the
        // env lock is still held.
        let _ = std::env::set_current_dir(&self.orig_cwd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A poisoned lock must still be acquirable: this is the whole point of
    /// [`env_lock`]'s `unwrap_or_else`. Tested against a private mutex rather
    /// than `ENV_MUTEX`, so proving it costs the rest of the suite nothing.
    ///
    /// This covers the *delegate*, not the delegation — see
    /// [`the_real_env_lock_tolerates_poisoning`], which covers `env_lock`
    /// itself. Reverting `env_lock` alone to `.unwrap()` left 1281 tests green
    /// (#372 review): a proxy for the property is not the property.
    ///
    /// Mutation this catches: `lock_ignoring_poison`'s body reverted to
    /// `mutex.lock().unwrap()` — this test then panics with `PoisonError`
    /// instead of returning.
    /// The property the whole module exists for, asserted on the **real**
    /// `env_lock()` rather than through a stand-in. Once the tolerance is in
    /// place, poisoning `ENV_MUTEX` is precisely harmless — which is what
    /// makes this test affordable, and what makes it fail loudly across the
    /// suite if the tolerance ever regresses.
    #[test]
    fn the_real_env_lock_tolerates_poisoning() {
        let panicked = std::panic::catch_unwind(|| {
            let _held = env_lock();
            panic!("deliberate panic while holding the shared env lock");
        });
        assert!(panicked.is_err(), "the helper panic must have unwound");
        assert!(
            ENV_MUTEX.lock().is_err(),
            "ENV_MUTEX must actually be poisoned now, otherwise this test proves nothing"
        );
        let _recovered = env_lock();
    }

    #[test]
    fn a_poisoned_mutex_is_still_acquirable() {
        static POISONED: std::sync::LazyLock<Mutex<u32>> =
            std::sync::LazyLock::new(|| Mutex::new(0));

        // Poison it exactly the way a legitimately failing test does: panic
        // while holding the guard.
        let panicked = std::panic::catch_unwind(|| {
            let mut held = lock_ignoring_poison(&POISONED);
            *held = 7;
            panic!("deliberate panic while holding the lock");
        });
        assert!(panicked.is_err(), "the helper panic must have unwound");
        assert!(
            POISONED.lock().is_err(),
            "the mutex must actually be poisoned, otherwise this test proves nothing"
        );

        let recovered = lock_ignoring_poison(&POISONED);
        assert_eq!(
            *recovered, 7,
            "the guard must be recovered from the poison, carrying the value the \
             panicking holder left"
        );
    }

    /// Read process-global state that other tests temporarily mutate, at a
    /// moment no guard holds it — i.e. under the lock, where by construction
    /// every guard has already restored what it changed. Reading it unguarded
    /// races: the reader can observe another test's temp dir as if it were the
    /// baseline, and then "restored" fails against a value that was never the
    /// original. (Measured: that is exactly how the first version of the two
    /// tests below failed in the full suite while passing in isolation.)
    fn undisturbed<T>(read: impl FnOnce() -> T) -> T {
        let _lock = env_lock();
        read()
    }

    /// The guard's contract is that it restores what it changed — including
    /// after the panic that poisoned the lock in the first place.
    ///
    /// Mutation this catches: deleting either arm of `IsolatedConfigDir`'s
    /// `Drop` (the `set_var` restore or the `remove_var` one).
    #[test]
    fn the_config_dir_guard_restores_the_previous_value() {
        let before = undisturbed(|| std::env::var("ARMADAI_CONFIG_DIR").ok());
        {
            let guard = IsolatedConfigDir::enter();
            assert_eq!(
                std::env::var("ARMADAI_CONFIG_DIR").ok().as_deref(),
                guard.config_dir().to_str(),
                "the guard must point ARMADAI_CONFIG_DIR at its own temp dir"
            );
            assert_ne!(
                Some(guard.config_dir().to_string_lossy().to_string()),
                before,
                "the temp dir must differ from whatever was set before"
            );
        }
        assert_eq!(
            undisturbed(|| std::env::var("ARMADAI_CONFIG_DIR").ok()),
            before,
            "ARMADAI_CONFIG_DIR must be exactly as it was, present or absent"
        );
    }

    /// Mutation this catches: deleting `IsolatedProjectDir`'s `Drop` (or its
    /// `set_current_dir` call), which leaves the whole test binary sitting in
    /// a deleted temp dir.
    #[test]
    fn the_project_dir_guard_restores_the_previous_cwd() {
        let before = undisturbed(|| std::env::current_dir().unwrap());
        let project = tempfile::tempdir().unwrap();
        // macOS puts temp dirs under a symlinked /var, so compare canonical
        // paths — `set_current_dir` resolves the link, the fixture path does
        // not.
        let expected = project.path().canonicalize().unwrap();
        {
            let _guard = IsolatedProjectDir::enter(project.path());
            assert_eq!(
                std::env::current_dir().unwrap().canonicalize().unwrap(),
                expected,
                "the guard must move the process into the project root"
            );
        }
        assert_eq!(
            undisturbed(|| std::env::current_dir().unwrap()),
            before,
            "the cwd must be back where it started"
        );
    }

    /// `with_var` must set what it is given, unset what it is given `None`
    /// for, and put both back — a variable that was present before, and one
    /// that was not.
    ///
    /// Mutation this catches: dropping the `saved_vars` loop from
    /// `IsolatedConfigDir`'s `Drop`, which leaks the test's `*_API_KEY` into
    /// every test that runs afterwards.
    #[test]
    fn extra_env_vars_are_set_then_restored_present_or_absent() {
        const PRESENT: &str = "ARMADAI_TEST_SUPPORT_PRESENT";
        const ABSENT: &str = "ARMADAI_TEST_SUPPORT_ABSENT";

        // A value that exists before the guard, so restoration has something
        // to put back rather than merely something to remove.
        {
            let _lock = env_lock();
            // SAFETY: serialised via the lock held above.
            unsafe {
                std::env::set_var(PRESENT, "original");
                std::env::remove_var(ABSENT);
            }
        }

        {
            let guard = IsolatedConfigDir::enter()
                .with_var(PRESENT, Some("overridden"))
                .with_var(ABSENT, Some("invented"));
            assert_eq!(std::env::var(PRESENT).as_deref(), Ok("overridden"));
            assert_eq!(std::env::var(ABSENT).as_deref(), Ok("invented"));
            drop(guard);
        }

        let (present, absent) =
            undisturbed(|| (std::env::var(PRESENT).ok(), std::env::var(ABSENT).ok()));
        assert_eq!(
            present.as_deref(),
            Some("original"),
            "a variable that existed before must come back with its own value"
        );
        assert_eq!(
            absent, None,
            "a variable invented by the guard must be gone again"
        );

        // And `None` really unsets for the guard's lifetime.
        {
            let _lock = env_lock();
            // SAFETY: serialised via the lock held above.
            unsafe { std::env::set_var(PRESENT, "original") };
        }
        {
            let _guard = IsolatedConfigDir::enter().with_var(PRESENT, None);
            assert!(
                std::env::var(PRESENT).is_err(),
                "`None` must remove the variable, not set it to the empty string"
            );
        }
        assert_eq!(
            undisturbed(|| std::env::var(PRESENT).ok()).as_deref(),
            Some("original")
        );
        {
            let _lock = env_lock();
            // SAFETY: serialised via the lock held above.
            unsafe { std::env::remove_var(PRESENT) };
        }
    }
}
