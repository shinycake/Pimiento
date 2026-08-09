//! §1 discovery: locate the user's existing `omp` binary, capture the
//! login-shell environment for spawn inheritance, and probe `--version`.
//!
//! Design notes (PLAN.md §1):
//!
//! * Precedence: `PIMIENTO_OMP_BIN` (absolute), then app setting, then
//!   `command -v omp` resolved in the *login shell* (`$SHELL -lc`).
//! * macOS GUI processes inherit a stripped environment; running the
//!   login shell recovers the user's real `PATH`, provider API-key vars,
//!   and shim locations (mise, Homebrew, `~/.local/bin`).
//! * The returned environment is what callers spawn the child `omp` with.
//!   Discovery never writes to `~/.omp` and never mutates process env.
//! * Testability: all external effects go through [`CommandRunner`], so
//!   tests inject scripts + fixed environments without touching the
//!   developer's shell.
//!
//! The shell script is a compile-time constant — no user data is
//! interpolated — and uses a collision-resistant marker line to separate
//! `command -v omp` output from the environment dump. `env -0` is
//! preferred so values containing newlines survive intact; a
//! newline-oriented `env` fallback keeps the code working on shells
//! whose `env` lacks `-0`.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::RpcError;

/// Inclusive lower bound of the documented tested OMP range.
///
/// Versions outside [`MIN_SUPPORTED`]..=[`MAX_SUPPORTED`] are still accepted;
/// the app surfaces an "outside tested range" banner elsewhere (PLAN.md §1,
/// "version gate, not version bundling").
pub const MIN_SUPPORTED: OmpVersion = OmpVersion {
    major: 17,
    minor: 2,
    patch: 10,
};

/// Inclusive upper bound of the documented tested OMP range.
pub const MAX_SUPPORTED: OmpVersion = OmpVersion {
    major: 17,
    minor: 2,
    patch: 11,
};

/// Marker line separating `command -v omp` from the environment dump in
/// the login-shell capture script. Long random suffix keeps collisions
/// with real env values astronomically unlikely.
const ENV_MARKER: &str = "--PIMIENTO-OMP-ENV-8f3c1a94b06d4e2a-BOUNDARY--";

/// Compile-time constant script — no user data interpolated. `command -v`
/// prints the resolved `omp` (empty + nonzero exit if not found, which we
/// tolerate so we can still return the captured env for diagnostics).
/// `env -0` emits NUL-terminated `KEY=VALUE` records; the fallback is
/// newline-delimited for shells without `-0`.
const LOGIN_SHELL_SCRIPT: &str = "\
command -v omp || true
printf '\\n%s\\n' '--PIMIENTO-OMP-ENV-8f3c1a94b06d4e2a-BOUNDARY--'
env -0 2>/dev/null || env
";

/// Parsed semantic version of the peer `omp` binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OmpVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl OmpVersion {
    /// Parse a version banner. Accepts anything containing an
    /// `omp/MAJOR.MINOR.PATCH` token (the `--version` output shape); any
    /// trailing build/date metadata is ignored.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError::Discovery`] if the input contains no
    /// `omp/<version>` token, if a numeric component is missing or
    /// non-numeric, or if the token has more than three dotted
    /// components.
    pub fn parse(text: &str) -> Result<Self, RpcError> {
        let needle = "omp/";
        let start = text.find(needle).ok_or_else(|| RpcError::Discovery {
            detail: format!("could not find `omp/<version>` token in: {text:?}"),
        })?;
        let rest = &text[start + needle.len()..];
        let tail_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let dotted = &rest[..tail_end];
        let mut parts = dotted.split('.');
        let major = parse_component(parts.next(), dotted)?;
        let minor = parse_component(parts.next(), dotted)?;
        let patch = parse_component(parts.next(), dotted)?;
        if parts.next().is_some() {
            return Err(RpcError::Discovery {
                detail: format!("expected MAJOR.MINOR.PATCH, got {dotted:?}"),
            });
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// Classify against the inclusive tested range
    /// [`MIN_SUPPORTED`]..=[`MAX_SUPPORTED`]. Outside versions are accepted
    /// with an app-layer warning.
    #[must_use]
    pub fn support(self) -> VersionSupport {
        if self < MIN_SUPPORTED {
            VersionSupport::BelowMinimum
        } else if self > MAX_SUPPORTED {
            VersionSupport::Newer
        } else {
            VersionSupport::Supported
        }
    }
}

impl fmt::Display for OmpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_component(part: Option<&str>, dotted: &str) -> Result<u32, RpcError> {
    let raw = part.ok_or_else(|| RpcError::Discovery {
        detail: format!("expected MAJOR.MINOR.PATCH, got {dotted:?}"),
    })?;
    raw.parse::<u32>().map_err(|_| RpcError::Discovery {
        detail: format!("non-numeric version component {raw:?} in {dotted:?}"),
    })
}

/// Support classification of a probed OMP against the tested range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSupport {
    /// Inside the inclusive tested range [`MIN_SUPPORTED`]..=[`MAX_SUPPORTED`].
    Supported,
    /// Above [`MAX_SUPPORTED`] — accept, warn at the app layer.
    Newer,
    /// Below [`MIN_SUPPORTED`] — accept, warn at the app layer.
    BelowMinimum,
}

/// Outcome of the discovery chain. `env` is the captured login-shell
/// environment; callers pass it verbatim to the spawned `omp` child.
#[derive(Debug, Clone)]
pub struct DiscoveredOmp {
    pub path: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub version_text: String,
    pub version: OmpVersion,
}

/// Inputs to [`discover`]. Everything the caller controls lives here so
/// tests can drive discovery deterministically without touching the real
/// process environment.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryInputs {
    /// Value of `PIMIENTO_OMP_BIN` (absolute path override), if set.
    pub override_bin: Option<PathBuf>,
    /// App-setting `omp_bin` (absolute path), if the user configured one.
    pub setting_bin: Option<PathBuf>,
    /// Login shell to invoke for env capture (e.g. `/bin/zsh`).
    /// `None` disables shell fallback, which is only useful for tests
    /// exercising the override/setting branches in isolation.
    pub login_shell: Option<PathBuf>,
    /// Environment to attribute to the override / setting branches (the
    /// current process env). The shell-capture branch replaces this with
    /// the captured login-shell environment.
    pub current_env: BTreeMap<OsString, OsString>,
}

/// Captured output of an external command.
#[derive(Debug, Clone)]
pub struct CapturedOutput {
    pub status_success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Injectable side-effect boundary. All external commands go through
/// this trait so tests never depend on the developer's `$SHELL` or on
/// whichever `omp` happens to be installed.
pub trait CommandRunner {
    /// Run `shell -lc <script>` and capture stdout/stderr/status.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError::Discovery`] wrapping any spawn or I/O failure
    /// so callers can attribute the failure to discovery.
    fn run_login_shell(&self, shell: &OsStr, script: &str) -> Result<CapturedOutput, RpcError>;

    /// Run `<program> --version` with the given environment (no
    /// inherited process env).
    ///
    /// # Errors
    ///
    /// Returns [`RpcError::Discovery`] wrapping any spawn or I/O failure
    /// so callers can attribute the failure to discovery.
    fn run_version(
        &self,
        program: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CapturedOutput, RpcError>;
}

/// Real-process implementation of [`CommandRunner`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run_login_shell(&self, shell: &OsStr, script: &str) -> Result<CapturedOutput, RpcError> {
        let output = Command::new(shell)
            .arg("-lc")
            .arg(script)
            .output()
            .map_err(|e| RpcError::Discovery {
                detail: format!("failed to spawn login shell {}: {e}", shell.display()),
            })?;
        Ok(CapturedOutput {
            status_success: output.status.success(),
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn run_version(
        &self,
        program: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CapturedOutput, RpcError> {
        let mut cmd = Command::new(program);
        cmd.arg("--version").env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        let output = cmd.output().map_err(|e| RpcError::Discovery {
            detail: format!("failed to run `{} --version`: {e}", program.display()),
        })?;
        Ok(CapturedOutput {
            status_success: output.status.success(),
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Run the full discovery chain.
///
/// Errors are meaningful [`RpcError`] variants; the caller decides which
/// map to a re-detect card vs. a hard failure.
///
/// # Errors
///
/// Propagates any [`RpcError`] produced while resolving the binary,
/// capturing the login-shell environment, or probing `--version`.
pub fn discover<R: CommandRunner>(
    inputs: &DiscoveryInputs,
    runner: &R,
) -> Result<DiscoveredOmp, RpcError> {
    let (path, env) = resolve_binary(inputs, runner)?;
    let (version_text, version) = probe_version(&path, &env, runner)?;
    Ok(DiscoveredOmp {
        path,
        env,
        version_text,
        version,
    })
}

fn resolve_binary<R: CommandRunner>(
    inputs: &DiscoveryInputs,
    runner: &R,
) -> Result<(PathBuf, BTreeMap<OsString, OsString>), RpcError> {
    if let Some(override_bin) = &inputs.override_bin {
        validate_absolute_file(override_bin, "PIMIENTO_OMP_BIN")?;
        return Ok((override_bin.clone(), inputs.current_env.clone()));
    }
    if let Some(setting_bin) = &inputs.setting_bin {
        validate_absolute_file(setting_bin, "app setting `omp_bin`")?;
        return Ok((setting_bin.clone(), inputs.current_env.clone()));
    }
    let shell = inputs
        .login_shell
        .as_deref()
        .ok_or_else(|| RpcError::Discovery {
            detail: "no login shell configured and neither override nor setting supplied".into(),
        })?;
    let captured = runner.run_login_shell(shell.as_os_str(), LOGIN_SHELL_SCRIPT)?;
    let (resolved, env) = parse_login_shell_output(&captured)?;
    let path = resolved.ok_or_else(|| RpcError::Discovery {
        detail: "`omp` not found on the login-shell PATH (install: `curl -fsSL https://omp.sh/install | sh`)".into(),
    })?;
    Ok((path, env))
}

fn validate_absolute_file(path: &Path, source: &str) -> Result<(), RpcError> {
    if !path.is_absolute() {
        return Err(RpcError::Discovery {
            detail: format!("{source} must be an absolute path, got {}", path.display()),
        });
    }
    let meta = std::fs::metadata(path).map_err(|e| RpcError::Discovery {
        detail: format!("{source}: cannot stat {}: {e}", path.display()),
    })?;
    if !meta.is_file() {
        return Err(RpcError::Discovery {
            detail: format!("{source}: {} is not a regular file", path.display()),
        });
    }
    Ok(())
}

/// Parse the stdout of [`LOGIN_SHELL_SCRIPT`] into `(path, env)`.
///
/// Format:
/// ```text
/// <command -v omp output — possibly empty>
///
/// --PIMIENTO-OMP-ENV-…-BOUNDARY--
/// <env dump, NUL-separated if `env -0` worked, else newline-separated>
/// ```
pub(crate) fn parse_login_shell_output(
    captured: &CapturedOutput,
) -> Result<(Option<PathBuf>, BTreeMap<OsString, OsString>), RpcError> {
    let marker_bytes = ENV_MARKER.as_bytes();
    let stdout = &captured.stdout;

    let marker_at = find_subslice(stdout, marker_bytes).ok_or_else(|| RpcError::Discovery {
        detail: format!(
            "login-shell capture missing env marker; exit={:?}, stderr={:?}",
            captured.exit_code,
            String::from_utf8_lossy(&captured.stderr),
        ),
    })?;

    let head = &stdout[..marker_at];
    // First non-empty stdout line before the marker is the `command -v omp` result.
    let resolved = head
        .split(|b| *b == b'\n')
        .map(trim_ascii_space)
        .find(|line| !line.is_empty())
        .filter(|line| line != &marker_bytes)
        .map(|line| PathBuf::from(os_from_bytes(line).into_owned()));

    // Env body: everything after the marker line's own trailing newline.
    let after_marker_start = marker_at + marker_bytes.len();
    let env_body = match stdout[after_marker_start..]
        .iter()
        .position(|b| *b == b'\n')
    {
        Some(nl) => &stdout[after_marker_start + nl + 1..],
        None => &[][..],
    };

    let env = if env_body.contains(&0) {
        parse_env_records(env_body, b'\0')
    } else {
        parse_env_records(env_body, b'\n')
    };

    Ok((resolved, env))
}

fn parse_env_records(body: &[u8], sep: u8) -> BTreeMap<OsString, OsString> {
    let mut out = BTreeMap::new();
    for record in body.split(|b| *b == sep) {
        if record.is_empty() {
            continue;
        }
        let Some(eq) = record.iter().position(|b| *b == b'=') else {
            continue;
        };
        let key = &record[..eq];
        if key.is_empty() {
            continue;
        }
        let value = &record[eq + 1..];
        out.insert(
            os_from_bytes(key).into_owned(),
            os_from_bytes(value).into_owned(),
        );
    }
    out
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn trim_ascii_space(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && matches!(s[start], b' ' | b'\t' | b'\r') {
        start += 1;
    }
    while end > start && matches!(s[end - 1], b' ' | b'\t' | b'\r') {
        end -= 1;
    }
    &s[start..end]
}

fn probe_version<R: CommandRunner>(
    path: &Path,
    env: &BTreeMap<OsString, OsString>,
    runner: &R,
) -> Result<(String, OmpVersion), RpcError> {
    let out = runner.run_version(path, env)?;
    if !out.status_success {
        return Err(RpcError::Discovery {
            detail: format!(
                "`{} --version` exited with {:?}; stderr: {}",
                path.display(),
                out.exit_code,
                String::from_utf8_lossy(&out.stderr),
            ),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let version = OmpVersion::parse(&text)?;
    Ok((text, version))
}

/// Lossy `OsStr` conversion for shell-output byte slices. Unix uses the
/// raw bytes; other platforms fall back to UTF-8 lossy (discovery is
/// unix-first per PLAN.md §1).
#[cfg(unix)]
fn os_from_bytes(bytes: &[u8]) -> std::borrow::Cow<'_, OsStr> {
    use std::os::unix::ffi::OsStrExt;
    std::borrow::Cow::Borrowed(OsStr::from_bytes(bytes))
}
#[cfg(not(unix))]
fn os_from_bytes(bytes: &[u8]) -> std::borrow::Cow<'_, OsStr> {
    std::borrow::Cow::Owned(std::ffi::OsString::from(
        String::from_utf8_lossy(bytes).into_owned(),
    ))
}
