//! Host-side read-only git chrome for the Context inspector.
//!
//! Mirrors OMP TUI status-line usefulness (branch, dirty counts, worktree,
//! ahead/behind). This is **not** OMP authority — omit the section when cwd
//! is not a git work tree or when `git` fails.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct GitInspectorInfo {
    pub(crate) branch_or_detached: String,
    pub(crate) unstaged: u32,
    pub(crate) staged: u32,
    pub(crate) untracked: u32,
    pub(crate) ahead: Option<u32>,
    pub(crate) behind: Option<u32>,
    pub(crate) worktree_label: Option<String>,
}

impl GitInspectorInfo {
    /// Compact status-line style summary, e.g. `main *2 +1 ?3 ↑1`.
    pub(crate) fn summary_line(&self) -> String {
        let mut parts = vec![self.branch_or_detached.clone()];
        if self.unstaged > 0 {
            parts.push(format!("*{}", self.unstaged));
        }
        if self.staged > 0 {
            parts.push(format!("+{}", self.staged));
        }
        if self.untracked > 0 {
            parts.push(format!("?{}", self.untracked));
        }
        if let Some(ahead) = self.ahead.filter(|n| *n > 0) {
            parts.push(format!("↑{ahead}"));
        }
        if let Some(behind) = self.behind.filter(|n| *n > 0) {
            parts.push(format!("↓{behind}"));
        }
        parts.join(" ")
    }
}

fn git_in(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Returns `None` when cwd is not inside a work tree or git is unavailable.
pub(crate) fn probe_git_inspector(cwd: &Path) -> Option<GitInspectorInfo> {
    let inside = git_in(cwd, &["rev-parse", "--is-inside-work-tree"])?;
    if inside != "true" {
        return None;
    }

    let branch_or_detached = match git_in(cwd, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        Some(branch) if !branch.is_empty() => branch,
        _ => {
            let short = git_in(cwd, &["rev-parse", "--short", "HEAD"])
                .unwrap_or_else(|| "detached".to_owned());
            format!("detached@{short}")
        }
    };

    let mut unstaged = 0u32;
    let mut staged = 0u32;
    let mut untracked = 0u32;
    if let Some(porcelain) = git_in(cwd, &["status", "--porcelain", "-uall"]) {
        for line in porcelain.lines() {
            if line.len() < 2 {
                continue;
            }
            let (index, worktree) = (
                line.as_bytes().first().copied().unwrap_or(b' '),
                line.as_bytes().get(1).copied().unwrap_or(b' '),
            );
            if index == b'?' && worktree == b'?' {
                untracked += 1;
                continue;
            }
            if index != b' ' && index != b'?' {
                staged += 1;
            }
            if worktree != b' ' && worktree != b'?' {
                unstaged += 1;
            }
        }
    }

    let (ahead, behind) = parse_ahead_behind(
        git_in(
            cwd,
            &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
        )
        .as_deref(),
    );

    let worktree_label = linked_worktree_label(cwd);

    Some(GitInspectorInfo {
        branch_or_detached,
        unstaged,
        staged,
        untracked,
        ahead,
        behind,
        worktree_label,
    })
}

fn parse_ahead_behind(raw: Option<&str>) -> (Option<u32>, Option<u32>) {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    // `rev-list --left-right --count A...B` prints "<behind>\t<ahead>" for A=upstream B=HEAD
    // when args are `@{upstream}...HEAD`: left = upstream-only (behind), right = HEAD-only (ahead).
    let mut parts = raw.split_whitespace();
    let behind = parts.next().and_then(|s| s.parse().ok());
    let ahead = parts.next().and_then(|s| s.parse().ok());
    (ahead, behind)
}

fn linked_worktree_label(cwd: &Path) -> Option<String> {
    let git_dir = git_in(cwd, &["rev-parse", "--git-dir"])?;
    let git_path = PathBuf::from(&git_dir);
    let git_path = if git_path.is_absolute() {
        git_path
    } else {
        cwd.join(git_path)
    };
    // Linked worktrees use `<common>/.git/worktrees/<name>` and a `.git` file in the worktree.
    let common = git_in(cwd, &["rev-parse", "--git-common-dir"])?;
    let common_path = {
        let p = PathBuf::from(&common);
        if p.is_absolute() { p } else { cwd.join(p) }
    };
    let canonical_git = std::fs::canonicalize(&git_path).ok()?;
    let canonical_common = std::fs::canonicalize(&common_path).ok()?;
    if canonical_git == canonical_common {
        return None;
    }
    let worktree_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("worktree");
    let project = common_path
        .parent() // strip `.git`
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    Some(format!("{project}/{worktree_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_line_hides_zero_counts() {
        let info = GitInspectorInfo {
            branch_or_detached: "main".into(),
            unstaged: 2,
            staged: 0,
            untracked: 1,
            ahead: Some(0),
            behind: Some(3),
            worktree_label: None,
        };
        assert_eq!(info.summary_line(), "main *2 ?1 ↓3");
    }

    #[test]
    fn parse_ahead_behind_counts() {
        assert_eq!(parse_ahead_behind(Some("2\t5")), (Some(5), Some(2)));
        assert_eq!(parse_ahead_behind(None), (None, None));
    }
}
