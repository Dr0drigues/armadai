//! Git checkout lookup, read by hand (no git process): it runs on every delegation.

use std::path::{Component, Path, PathBuf};

/// Lexical `..`/`.` removal, for a path whose base is already canonical (or absolute).
fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// The `gitdir:` value of the file `dot_git` (in folder `d`), resolved to a path.
fn gitdir_of_file(d: &Path, dot_git: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(dot_git).ok()?;
    let gitdir = raw.lines().next()?.strip_prefix("gitdir:")?.trim();
    // A relative value is relative to the real folder: reached through a symbolic
    // link, a lexical `..` would climb the link's parent instead.
    let base = if Path::new(gitdir).is_absolute() {
        d.to_path_buf()
    } else {
        std::fs::canonicalize(d).ok()?
    };
    Some(clean(&base.join(gitdir)))
}

/// The git directory of the checkout holding `dir`: `<d>/.git` when it is a folder,
/// the `gitdir:` value of `<d>/.git` resolved against `<d>` when it is a file
/// (git writes it relative with `worktree.useRelativePaths`). `d` is `dir` or its
/// nearest ancestor holding a `.git`.
pub(crate) fn git_dir_of(dir: &Path) -> Option<PathBuf> {
    for d in dir.ancestors() {
        let dot_git = d.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            return gitdir_of_file(d, &dot_git);
        }
    }
    None
}

/// For a caller that must not guess: the nearest ancestor of `dir` with a `.git` (of any
/// kind, a dangling link included) decides alone, never a `.git` further up. `Ok(None)`
/// outside git; `Ok(Some)` when that `.git` is a folder, or a file whose `gitdir:` leads to
/// an existing folder; `Err(that .git)` otherwise (empty file, no `gitdir:` line, dangling
/// link, `gitdir:` to a folder that is gone, which `git worktree repair` mends).
pub(crate) fn git_dir_checked(dir: &Path) -> Result<Option<PathBuf>, PathBuf> {
    let Some((d, dot_git)) = dir
        .ancestors()
        .map(|d| (d, d.join(".git")))
        .find(|(_, g)| g.symlink_metadata().is_ok())
    else {
        return Ok(None);
    };
    let gd = if dot_git.is_dir() {
        Some(dot_git.clone())
    } else if dot_git.is_file() {
        gitdir_of_file(d, &dot_git).filter(|g| g.is_dir())
    } else {
        None
    };
    gd.map(Some).ok_or(dot_git)
}

/// The main checkout behind a linked worktree; `None` in a main checkout, in a
/// submodule or outside git. A worktree's git dir is `<repo>/.git/worktrees/<name>`.
pub(crate) fn main_checkout_of(dir: &Path) -> Option<PathBuf> {
    let gd = git_dir_of(dir)?;
    let parts: Vec<_> = gd.components().collect();
    let i = parts.len().checked_sub(3)?;
    if parts[i].as_os_str() != ".git" || parts[i + 1].as_os_str() != "worktrees" {
        return None;
    }
    Some(parts[..i].iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dotgit(dir: &Path, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(".git"), content).unwrap();
    }

    #[test]
    fn a_git_folder_is_its_own_git_dir_and_not_a_worktree() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("m/.git")).unwrap();
        assert_eq!(
            git_dir_of(&t.path().join("m")),
            Some(t.path().join("m/.git"))
        );
        assert_eq!(main_checkout_of(&t.path().join("m")), None);
    }

    #[test]
    fn git_dir_checked_tells_outside_git_from_a_dot_git_that_leads_nowhere() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        // a folder
        fs::create_dir_all(root.join("m/.git")).unwrap();
        assert_eq!(
            git_dir_checked(&root.join("m")),
            Ok(Some(root.join("m/.git")))
        );
        // a valid `gitdir:` file, from a subfolder
        let gd = root.join("main/.git/worktrees/wt");
        fs::create_dir_all(&gd).unwrap();
        dotgit(&root.join("wt"), &format!("gitdir: {}\n", gd.display()));
        fs::create_dir_all(root.join("wt/src")).unwrap();
        assert_eq!(git_dir_checked(&root.join("wt/src")), Ok(Some(gd)));
        // an empty file and a file without a `gitdir:` line
        dotgit(&root.join("empty"), "");
        assert_eq!(
            git_dir_checked(&root.join("empty")),
            Err(root.join("empty/.git"))
        );
        dotgit(&root.join("junk"), "not a gitdir line\n");
        assert_eq!(
            git_dir_checked(&root.join("junk")),
            Err(root.join("junk/.git"))
        );
        // outside git
        fs::create_dir_all(root.join("free")).unwrap();
        assert_eq!(git_dir_checked(&root.join("free")), Ok(None));
    }

    #[cfg(unix)]
    #[test]
    fn git_dir_checked_never_climbs_past_the_nearest_dot_git() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        fs::create_dir_all(root.join("main/.git")).unwrap();
        // a dangling `.git` under a folder that has a valid one
        fs::create_dir_all(root.join("main/dang")).unwrap();
        std::os::unix::fs::symlink(root.join("nowhere"), root.join("main/dang/.git")).unwrap();
        assert_eq!(
            git_dir_checked(&root.join("main/dang")),
            Err(root.join("main/dang/.git"))
        );
        // a `gitdir:` to a folder that is gone
        dotgit(
            &root.join("main/gone"),
            &format!(
                "gitdir: {}\n",
                root.join("absent/.git/worktrees/gone").display()
            ),
        );
        assert_eq!(
            git_dir_checked(&root.join("main/gone/src")),
            Err(root.join("main/gone/.git"))
        );
        // a `gitdir:` to a folder that exists
        let gd = root.join("main/.git/worktrees/ok");
        fs::create_dir_all(&gd).unwrap();
        dotgit(
            &root.join("main/ok"),
            &format!("gitdir: {}\n", gd.display()),
        );
        assert_eq!(git_dir_checked(&root.join("main/ok")), Ok(Some(gd)));
        // control: the folder above is still found when nothing is in between
        fs::create_dir_all(root.join("main/plain")).unwrap();
        assert_eq!(
            git_dir_checked(&root.join("main/plain")),
            Ok(Some(root.join("main/.git")))
        );
    }

    #[test]
    fn an_absolute_gitdir_file_gives_the_main_checkout() {
        let t = tempfile::tempdir().unwrap();
        let gd = t.path().join("main/.git/worktrees/wt");
        dotgit(&t.path().join("wt"), &format!("gitdir: {}\n", gd.display()));
        assert_eq!(git_dir_of(&t.path().join("wt")), Some(gd));
        assert_eq!(
            main_checkout_of(&t.path().join("wt")),
            Some(t.path().join("main"))
        );
    }

    #[test]
    fn a_relative_gitdir_is_resolved_against_the_dir_of_the_dot_git_file() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        dotgit(
            &root.join("wt-rel"),
            "gitdir: ../main/.git/worktrees/wt-rel\n",
        );
        assert_eq!(
            git_dir_of(&root.join("wt-rel")),
            Some(root.join("main/.git/worktrees/wt-rel"))
        );
        assert_eq!(
            main_checkout_of(&root.join("wt-rel")),
            Some(root.join("main"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_gitdir_is_resolved_from_the_real_folder_not_the_link() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        dotgit(
            &root.join("real/x/wt-rel"),
            "gitdir: ../main/.git/worktrees/wt-rel\n",
        );
        fs::create_dir_all(root.join("other")).unwrap();
        std::os::unix::fs::symlink(root.join("real/x/wt-rel"), root.join("other/wt-link")).unwrap();
        let via_link = root.join("other/wt-link");
        assert_eq!(
            git_dir_of(&via_link),
            Some(root.join("real/x/main/.git/worktrees/wt-rel"))
        );
        assert_eq!(main_checkout_of(&via_link), Some(root.join("real/x/main")));
    }

    #[test]
    fn a_subfolder_finds_the_checkout_above() {
        let t = tempfile::tempdir().unwrap();
        let gd = t.path().join("main/.git/worktrees/wt");
        dotgit(&t.path().join("wt"), &format!("gitdir: {}\n", gd.display()));
        let sub = t.path().join("wt/a/b");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(git_dir_of(&sub), Some(gd));
        assert_eq!(main_checkout_of(&sub), Some(t.path().join("main")));
    }

    #[test]
    fn outside_git_there_is_nothing() {
        let t = tempfile::tempdir().unwrap();
        assert!(t.path().ancestors().all(|a| !a.join(".git").exists()));
        assert_eq!(git_dir_of(t.path()), None);
        assert_eq!(main_checkout_of(t.path()), None);
    }

    #[test]
    fn a_submodule_is_not_a_worktree() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        dotgit(&root.join("main/msub"), "gitdir: ../.git/modules/msub\n");
        assert_eq!(
            git_dir_of(&root.join("main/msub")),
            Some(root.join("main/.git/modules/msub"))
        );
        assert_eq!(main_checkout_of(&root.join("main/msub")), None);
    }
}
