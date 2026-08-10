//! Explicit OMP update checks and installations through the user's existing
//! `omp` binary.
//!
//! Pimiento never writes OMP's files itself and never invokes the curl
//! installer. These helpers only run the official `omp update` commands when
//! called by an explicit product action.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use crate::discovery::{CommandRunner, OmpVersion};

const UNKNOWN_VERSION: &str = "unknown";
const DETAIL_SNIPPET_CHARS: usize = 1_024;

/// Result of asking OMP whether an update is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmpUpdateCheck {
    /// The installed OMP version is current.
    UpToDate {
        /// Installed version reported by OMP.
        current: String,
    },
    /// A newer OMP version is available.
    Available {
        /// Installed version reported by OMP.
        current: String,
        /// Latest version reported by OMP.
        latest: String,
    },
    /// The command failed or its output was not recognizable.
    ///
    /// Callers should not offer an update action for this result.
    Failed {
        /// Human-readable command or parsing failure.
        detail: String,
    },
}

/// Result of explicitly invoking OMP's official updater.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmpUpdateInstall {
    /// OMP reported a successful update.
    Updated {
        /// Version before the update, when OMP included it in its output.
        previous: Option<String>,
        /// Version after the update, re-probed from the replaced binary when
        /// possible.
        current: String,
        /// Combined stdout and stderr from `omp update`.
        raw: String,
    },
    /// OMP could not be updated.
    Failed {
        /// Human-readable command failure.
        detail: String,
        /// Combined stdout and stderr captured from `omp update`.
        raw: String,
    },
}

/// Parse captured `omp update --check` output.
///
/// ANSI CSI color sequences are ignored. An unsuccessful exit status always
/// produces [`OmpUpdateCheck::Failed`], even if stdout contains a success
/// marker.
#[must_use]
pub fn parse_update_check_output(stdout: &str, stderr: &str, success: bool) -> OmpUpdateCheck {
    let clean_stdout = strip_ansi(stdout);
    let clean_stderr = strip_ansi(stderr);

    if !success {
        return OmpUpdateCheck::Failed {
            detail: format!(
                "`omp update --check` failed: {}",
                format_output_detail(&clean_stdout, &clean_stderr)
            ),
        };
    }

    let current = value_after_marker(&clean_stdout, "Current version:")
        .unwrap_or_else(|| UNKNOWN_VERSION.to_owned());

    if clean_stdout.contains("Already up to date") {
        return OmpUpdateCheck::UpToDate { current };
    }

    if let Some(latest) = value_after_marker(&clean_stdout, "New version available:") {
        return OmpUpdateCheck::Available { current, latest };
    }

    OmpUpdateCheck::Failed {
        detail: format!(
            "could not parse `omp update --check` output: {}",
            format_output_detail(&clean_stdout, &clean_stderr)
        ),
    }
}

/// Run `omp update --check` with only the supplied environment.
///
/// Command transport failures are returned as values so update discovery never
/// panics.
#[must_use]
pub fn check_omp_update<R: CommandRunner + ?Sized>(
    program: &Path,
    env: &BTreeMap<OsString, OsString>,
    runner: &R,
) -> OmpUpdateCheck {
    match runner.run_omp(program, &["update", "--check"], env) {
        Ok(output) => parse_update_check_output(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status_success,
        ),
        Err(error) => OmpUpdateCheck::Failed {
            detail: format!("could not run `omp update --check`: {error}"),
        },
    }
}

/// Explicitly run `omp update` with only the supplied environment.
///
/// After a successful updater exit, this re-runs `omp --version` because the
/// updater replaces the binary on disk. A failed re-probe does not change a
/// successful update into a failure; `current` falls back to updater output or
/// `"unknown"`.
#[must_use]
pub fn install_omp_update<R: CommandRunner + ?Sized>(
    program: &Path,
    env: &BTreeMap<OsString, OsString>,
    runner: &R,
) -> OmpUpdateInstall {
    let output = match runner.run_omp(program, &["update"], env) {
        Ok(output) => output,
        Err(error) => {
            return OmpUpdateInstall::Failed {
                detail: format!("could not run `omp update`: {error}"),
                raw: String::new(),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = combine_output(&stdout, &stderr);
    let clean_stdout = strip_ansi(&stdout);
    let clean_stderr = strip_ansi(&stderr);

    if !output.status_success {
        return OmpUpdateInstall::Failed {
            detail: format!(
                "`omp update` failed: {}",
                format_output_detail(&clean_stdout, &clean_stderr)
            ),
            raw,
        };
    }

    let combined_clean = combine_output(&clean_stdout, &clean_stderr);
    let previous = value_after_marker(&combined_clean, "Current version:");
    let reported_current = value_after_marker(&combined_clean, "Updated to");
    let current = probe_current_version(program, env, runner)
        .or(reported_current)
        .unwrap_or_else(|| UNKNOWN_VERSION.to_owned());

    OmpUpdateInstall::Updated {
        previous,
        current,
        raw,
    }
}

fn probe_current_version<R: CommandRunner + ?Sized>(
    program: &Path,
    env: &BTreeMap<OsString, OsString>,
    runner: &R,
) -> Option<String> {
    let output = runner.run_version(program, env).ok()?;
    if !output.status_success {
        return None;
    }
    let banner = String::from_utf8_lossy(&output.stdout);
    OmpVersion::parse(&banner)
        .ok()
        .map(|version| version.to_string())
}

fn value_after_marker(text: &str, marker: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let marker_at = line.find(marker)?;
        let value = line[marker_at + marker.len()..].trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn combine_output(stdout: &str, stderr: &str) -> String {
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout.to_owned(),
        (true, false) => stderr.to_owned(),
        (true, true) => String::new(),
    }
}

fn format_output_detail(stdout: &str, stderr: &str) -> String {
    let stdout = snippet(stdout);
    let stderr = snippet(stderr);
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout: {stdout}; stderr: {stderr}"),
        (false, true) => format!("stdout: {stdout}"),
        (true, false) => format!("stderr: {stderr}"),
        (true, true) => "no output".to_owned(),
    }
}

fn snippet(text: &str) -> String {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let prefix: String = chars.by_ref().take(DETAIL_SNIPPET_CHARS).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

pub(crate) fn strip_ansi(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for sequence_character in chars.by_ref() {
                if ('@'..='~').contains(&sequence_character) {
                    break;
                }
            }
        } else {
            clean.push(character);
        }
    }

    clean
}
