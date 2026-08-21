//! Host-side read-only git chrome for the Context inspector.
//!
//! At-a-glance branch / sync / dirty / line-diff / HEAD / fetch age.
//! This is **not** OMP authority — omit the section when cwd is not a git
//! work tree or when `git` fails. `Render` only peeks a TTL cache; refresh
//! runs on a background task so inspector paints never wait on `git`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Inspector git is host chrome, not per-keystroke truth. Keep the TTL long
/// enough that streaming paints never wait on `git` on the UI thread.
const CACHE_TTL: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct GitDiffLines {
    pub(crate) insertions: u32,
    pub(crate) deletions: u32,
}

impl GitDiffLines {
    pub(crate) fn is_empty(&self) -> bool {
        self.insertions == 0 && self.deletions == 0
    }

    pub(crate) fn format_delta(&self) -> String {
        format!("+{} −{}", self.insertions, self.deletions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct GitInspectorInfo {
    pub(crate) branch_or_detached: String,
    pub(crate) upstream: Option<String>,
    pub(crate) remote: Option<String>,
    pub(crate) unstaged: u32,
    pub(crate) staged: u32,
    pub(crate) untracked: u32,
    pub(crate) conflicted: u32,
    pub(crate) ahead: Option<u32>,
    pub(crate) behind: Option<u32>,
    pub(crate) unstaged_diff: GitDiffLines,
    pub(crate) staged_diff: GitDiffLines,
    pub(crate) head_short: Option<String>,
    pub(crate) head_subject: Option<String>,
    pub(crate) stash_count: u32,
    /// Humanized age of `.git/FETCH_HEAD`, or `None` when never fetched.
    pub(crate) fetch_age: Option<String>,
    pub(crate) worktree_label: Option<String>,
}

impl GitInspectorInfo {
    pub(crate) fn is_clean(&self) -> bool {
        self.unstaged == 0 && self.staged == 0 && self.untracked == 0 && self.conflicted == 0
    }

    pub(crate) fn sync_line(&self) -> String {
        match (&self.upstream, self.ahead, self.behind) {
            (None, _, _) => "no upstream".to_owned(),
            (Some(up), ahead, behind) => {
                let a = ahead.unwrap_or(0);
                let b = behind.unwrap_or(0);
                if a == 0 && b == 0 {
                    format!("{up} · in sync")
                } else {
                    let mut bits = Vec::new();
                    if a > 0 {
                        bits.push(format!("↑{a} ahead"));
                    }
                    if b > 0 {
                        bits.push(format!("↓{b} behind"));
                    }
                    format!("{up} · {}", bits.join(" · "))
                }
            }
        }
    }

    pub(crate) fn working_tree_line(&self) -> String {
        if self.is_clean() {
            return "clean".to_owned();
        }
        let mut bits = Vec::new();
        if self.conflicted > 0 {
            bits.push(format!("{} conflicted", self.conflicted));
        }
        if self.staged > 0 {
            bits.push(format!("{} staged", self.staged));
        }
        if self.unstaged > 0 {
            bits.push(format!("{} modified", self.unstaged));
        }
        if self.untracked > 0 {
            bits.push(format!("{} untracked", self.untracked));
        }
        bits.join(" · ")
    }

    pub(crate) fn diff_line(&self) -> Option<String> {
        if self.unstaged_diff.is_empty() && self.staged_diff.is_empty() {
            return None;
        }
        let mut bits = Vec::new();
        if !self.unstaged_diff.is_empty() {
            bits.push(format!("{} working", self.unstaged_diff.format_delta()));
        }
        if !self.staged_diff.is_empty() {
            bits.push(format!("{} staged", self.staged_diff.format_delta()));
        }
        Some(bits.join(" · "))
    }

    pub(crate) fn head_line(&self) -> Option<String> {
        match (&self.head_short, &self.head_subject) {
            (Some(h), Some(s)) => Some(format!("{h} — {s}")),
            (Some(h), None) => Some(h.clone()),
            _ => None,
        }
    }
}

struct CacheEntry {
    cwd: PathBuf,
    at: Instant,
    info: Option<GitInspectorInfo>,
}

static CACHE: Mutex<Option<CacheEntry>> = Mutex::new(None);

/// Never runs `git`. Returns the last snapshot for `cwd`, even if stale.
///
/// Call [`git_inspector_needs_refresh`] from a view that can spawn a
/// background task; do not probe from `Render`.
pub(crate) fn probe_git_inspector(cwd: &Path) -> Option<GitInspectorInfo> {
    let Ok(guard) = CACHE.lock() else {
        return None;
    };
    guard
        .as_ref()
        .filter(|entry| entry.cwd == cwd)
        .and_then(|entry| entry.info.clone())
}

pub(crate) fn git_inspector_needs_refresh(cwd: &Path) -> bool {
    let Ok(guard) = CACHE.lock() else {
        return false;
    };
    match guard.as_ref() {
        Some(entry) if entry.cwd == cwd => entry.at.elapsed() >= CACHE_TTL,
        _ => true,
    }
}

pub(crate) fn refresh_git_inspector(cwd: &Path) -> Option<GitInspectorInfo> {
    let info = probe_git_inspector_uncached(cwd);
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(CacheEntry {
            cwd: cwd.to_path_buf(),
            at: Instant::now(),
            info: info.clone(),
        });
    }
    info
}

fn probe_git_inspector_uncached(cwd: &Path) -> Option<GitInspectorInfo> {
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

    let upstream = git_in(
        cwd,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    let remote = upstream
        .as_deref()
        .and_then(|up| up.split('/').next())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            git_in(cwd, &["remote"]).and_then(|list| {
                list.lines()
                    .next()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
        });

    let mut unstaged = 0u32;
    let mut staged = 0u32;
    let mut untracked = 0u32;
    let mut conflicted = 0u32;
    if let Some(porcelain) = git_in(cwd, &["status", "--porcelain", "-uall"]) {
        for line in porcelain.lines() {
            if line.len() < 2 {
                continue;
            }
            let (index, worktree) = (
                line.as_bytes().first().copied().unwrap_or(b' '),
                line.as_bytes().get(1).copied().unwrap_or(b' '),
            );
            if is_conflict(index, worktree) {
                conflicted += 1;
                continue;
            }
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

    let unstaged_diff = parse_numstat(git_in(cwd, &["diff", "--numstat"]).as_deref());
    let staged_diff = parse_numstat(git_in(cwd, &["diff", "--cached", "--numstat"]).as_deref());

    let (head_short, head_subject) =
        parse_head_oneline(git_in(cwd, &["log", "-1", "--format=%h\t%s"]).as_deref());

    let stash_count = git_in(cwd, &["stash", "list"]).map_or(0, |out| {
        u32::try_from(out.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(u32::MAX)
    });

    let fetch_age = fetch_head_age(cwd);
    let worktree_label = linked_worktree_label(cwd);

    Some(GitInspectorInfo {
        branch_or_detached,
        upstream,
        remote,
        unstaged,
        staged,
        untracked,
        conflicted,
        ahead,
        behind,
        unstaged_diff,
        staged_diff,
        head_short,
        head_subject,
        stash_count,
        fetch_age,
        worktree_label,
    })
}

fn is_conflict(index: u8, worktree: u8) -> bool {
    matches!(
        (index, worktree),
        (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D')
    )
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

fn parse_ahead_behind(raw: Option<&str>) -> (Option<u32>, Option<u32>) {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    // `rev-list --left-right --count @{upstream}...HEAD`:
    // left = upstream-only (behind), right = HEAD-only (ahead).
    let mut parts = raw.split_whitespace();
    let behind = parts.next().and_then(|s| s.parse().ok());
    let ahead = parts.next().and_then(|s| s.parse().ok());
    (ahead, behind)
}

fn parse_numstat(raw: Option<&str>) -> GitDiffLines {
    let mut insertions = 0u32;
    let mut deletions = 0u32;
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return GitDiffLines::default();
    };
    for line in raw.lines() {
        let mut cols = line.split('\t');
        let add = cols.next().unwrap_or("-");
        let del = cols.next().unwrap_or("-");
        // Binary files report `-`.
        if let Ok(n) = add.parse::<u32>() {
            insertions = insertions.saturating_add(n);
        }
        if let Ok(n) = del.parse::<u32>() {
            deletions = deletions.saturating_add(n);
        }
    }
    GitDiffLines {
        insertions,
        deletions,
    }
}

fn parse_head_oneline(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let mut parts = raw.splitn(2, '\t');
    let short = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let subject = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    (short, subject)
}

fn fetch_head_age(cwd: &Path) -> Option<String> {
    let path = git_in(cwd, &["rev-parse", "--git-path", "FETCH_HEAD"])?;
    let fetch_path = {
        let p = PathBuf::from(&path);
        if p.is_absolute() { p } else { cwd.join(p) }
    };
    let meta = std::fs::metadata(&fetch_path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    let modified = meta.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).ok()?;
    Some(humanize_age(age))
}

fn humanize_age(age: Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        "just now".to_owned()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86400 * 14 {
        format!("{}d ago", secs / 86400)
    } else {
        format!("{}w ago", secs / (86400 * 7))
    }
}

fn linked_worktree_label(cwd: &Path) -> Option<String> {
    let git_dir = git_in(cwd, &["rev-parse", "--git-dir"])?;
    let git_path = PathBuf::from(&git_dir);
    let git_path = if git_path.is_absolute() {
        git_path
    } else {
        cwd.join(git_path)
    };
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
    fn sync_line_in_sync_and_ahead_behind() {
        let mut info = GitInspectorInfo {
            upstream: Some("origin/main".into()),
            ahead: Some(0),
            behind: Some(0),
            ..Default::default()
        };
        assert_eq!(info.sync_line(), "origin/main · in sync");
        info.ahead = Some(2);
        info.behind = Some(1);
        assert_eq!(info.sync_line(), "origin/main · ↑2 ahead · ↓1 behind");
        info.upstream = None;
        assert_eq!(info.sync_line(), "no upstream");
    }

    #[test]
    fn working_tree_and_diff_lines() {
        let info = GitInspectorInfo {
            unstaged: 3,
            staged: 1,
            untracked: 1,
            unstaged_diff: GitDiffLines {
                insertions: 42,
                deletions: 17,
            },
            staged_diff: GitDiffLines {
                insertions: 12,
                deletions: 3,
            },
            ..Default::default()
        };
        assert_eq!(
            info.working_tree_line(),
            "1 staged · 3 modified · 1 untracked"
        );
        assert_eq!(
            info.diff_line().as_deref(),
            Some("+42 −17 working · +12 −3 staged")
        );
    }

    #[test]
    fn parse_numstat_sums_and_skips_binary() {
        let raw = "10\t2\ta.rs\n-\t-\tbin.png\n3\t1\tb.rs\n";
        assert_eq!(
            parse_numstat(Some(raw)),
            GitDiffLines {
                insertions: 13,
                deletions: 3,
            }
        );
    }

    #[test]
    fn parse_ahead_behind_counts() {
        assert_eq!(parse_ahead_behind(Some("2\t5")), (Some(5), Some(2)));
        assert_eq!(parse_ahead_behind(None), (None, None));
    }

    #[test]
    fn parse_head_oneline_splits_hash_and_subject() {
        assert_eq!(
            parse_head_oneline(Some("abc1234\tfix the rename modal")),
            (Some("abc1234".into()), Some("fix the rename modal".into()))
        );
    }

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(Duration::from_secs(12)), "just now");
        assert_eq!(humanize_age(Duration::from_mins(3)), "3m ago");
        assert_eq!(humanize_age(Duration::from_hours(2)), "2h ago");
        assert_eq!(humanize_age(Duration::from_hours(72)), "3d ago");
    }
}
