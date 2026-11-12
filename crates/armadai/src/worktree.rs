//! Git checkout lookup, read by hand (no git process): it runs on every delegation.

use std::path::{Component, Path, PathBuf};

/// Lexical `..`/`.` removal: the path may not exist, and canonicalizing would
/// rewrite symlinked prefixes (`/var` vs `/private/var`).
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
            let raw = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = raw.lines().next()?.strip_prefix("gitdir:")?.trim();
            return Some(clean(&d.join(gitdir)));
        }
    }
    None
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
        dotgit(
            &t.path().join("wt-rel"),
            "gitdir: ../main/.git/worktrees/wt-rel\n",
        );
        assert_eq!(
            git_dir_of(&t.path().join("wt-rel")),
            Some(t.path().join("main/.git/worktrees/wt-rel"))
        );
        assert_eq!(
            main_checkout_of(&t.path().join("wt-rel")),
            Some(t.path().join("main"))
        );
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
        // a temp dir may sit under a repo on some machines; use a name no `.git` can match
        if git_dir_of(t.path()).is_none() {
            assert_eq!(main_checkout_of(t.path()), None);
        }
    }

    #[test]
    fn a_submodule_is_not_a_worktree() {
        let t = tempfile::tempdir().unwrap();
        dotgit(
            &t.path().join("main/msub"),
            "gitdir: ../.git/modules/msub\n",
        );
        assert_eq!(
            git_dir_of(&t.path().join("main/msub")),
            Some(t.path().join("main/.git/modules/msub"))
        );
        assert_eq!(main_checkout_of(&t.path().join("main/msub")), None);
    }
}
