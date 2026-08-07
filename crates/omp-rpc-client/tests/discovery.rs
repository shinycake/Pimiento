//! §1 discovery tests — deterministic through the injectable
//! [`CommandRunner`] boundary. These never invoke the developer's
//! `$SHELL` or the real installed `omp`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use omp_rpc_client::discovery::{
    CapturedOutput, CommandRunner, DiscoveryInputs, MIN_SUPPORTED, OmpVersion, VersionSupport,
    discover,
};
use omp_rpc_client::error::RpcError;

// ---------- fixtures --------------------------------------------------

#[derive(Default)]
struct FakeRunner {
    /// Response for `run_login_shell(shell, script)`.
    shell_out: Option<CapturedOutput>,
    /// Keyed by binary path: version-command response.
    version_out: std::cell::RefCell<BTreeMap<PathBuf, CapturedOutput>>,
    calls: std::cell::RefCell<Vec<String>>,
}

impl FakeRunner {
    fn with_version(self, path: &Path, out: CapturedOutput) -> Self {
        self.version_out
            .borrow_mut()
            .insert(path.to_path_buf(), out);
        self
    }
}

impl CommandRunner for FakeRunner {
    fn run_login_shell(&self, shell: &OsStr, script: &str) -> Result<CapturedOutput, RpcError> {
        self.calls.borrow_mut().push(format!(
            "shell:{}:{}",
            shell.to_string_lossy(),
            script.len()
        ));
        self.shell_out.clone().ok_or_else(|| RpcError::Discovery {
            detail: "test: no shell_out configured".into(),
        })
    }

    fn run_version(
        &self,
        program: &Path,
        _env: &BTreeMap<OsString, OsString>,
    ) -> Result<CapturedOutput, RpcError> {
        self.calls
            .borrow_mut()
            .push(format!("version:{}", program.display()));
        self.version_out
            .borrow()
            .get(program)
            .cloned()
            .ok_or_else(|| RpcError::Discovery {
                detail: format!("test: no version_out for {}", program.display()),
            })
    }
}

fn ok(stdout: &str) -> CapturedOutput {
    CapturedOutput {
        status_success: true,
        exit_code: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

fn write_exec(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).expect("create fixture bin");
    // Body is irrelevant — discovery never executes it in these tests
    // (FakeRunner intercepts version probes).
    f.write_all(b"#!/bin/sh\nexit 0\n").expect("write");
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path)
            .expect("test fixture operation must succeed")
            .permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).expect("test fixture operation must succeed");
    }
    path
}

// ---------- OmpVersion ------------------------------------------------

#[test]
fn version_parses_installed_banner() {
    let v = OmpVersion::parse("omp/17.2.10").expect("test fixture operation must succeed");
    assert_eq!(v, MIN_SUPPORTED);
    assert_eq!(v.support(), VersionSupport::Supported);
    assert_eq!(v.to_string(), "17.2.10");
}

#[test]
fn version_parses_with_surrounding_text() {
    let v =
        OmpVersion::parse("omp/17.3.0 (build 2026-08-06)\n").expect("parse with build metadata");
    assert_eq!(v.major, 17);
    assert_eq!(v.minor, 3);
    assert_eq!(v.patch, 0);
    assert_eq!(v.support(), VersionSupport::Newer);
}

#[test]
fn version_rejects_missing_token() {
    let err = OmpVersion::parse("hello world").expect_err("test fixture operation must fail");
    assert!(matches!(err, RpcError::Discovery { .. }));
}

#[test]
fn version_rejects_non_numeric() {
    let err = OmpVersion::parse("omp/17.2.beta").expect_err("test fixture operation must fail");
    assert!(matches!(err, RpcError::Discovery { .. }));
}

#[test]
fn version_rejects_extra_component() {
    let err = OmpVersion::parse("omp/17.2.10.4").expect_err("test fixture operation must fail");
    assert!(matches!(err, RpcError::Discovery { .. }));
}

#[test]
fn version_below_minimum() {
    let v = OmpVersion::parse("omp/17.2.9").expect("test fixture operation must succeed");
    assert_eq!(v.support(), VersionSupport::BelowMinimum);
}

#[test]
fn version_newer_major_supported() {
    let v = OmpVersion::parse("omp/18.0.0").expect("test fixture operation must succeed");
    assert_eq!(v.support(), VersionSupport::Newer);
}

// ---------- precedence -----------------------------------------------

#[test]
fn override_wins_and_never_invokes_shell() {
    let dir = tempfile::tempdir().expect("test fixture operation must succeed");
    let bin = write_exec(dir.path(), "omp");
    let runner = FakeRunner::default().with_version(&bin, ok("omp/17.2.10\n"));

    let mut env = BTreeMap::new();
    env.insert(OsString::from("PATH"), OsString::from("/tmp"));
    let inputs = DiscoveryInputs {
        override_bin: Some(bin.clone()),
        setting_bin: Some(PathBuf::from("/does/not/exist/other-omp")),
        login_shell: Some(PathBuf::from("/bin/sh")),
        current_env: env.clone(),
    };

    let found = discover(&inputs, &runner).expect("test fixture operation must succeed");
    assert_eq!(found.path, bin);
    assert_eq!(found.version, MIN_SUPPORTED);
    assert_eq!(found.env, env);

    let calls = runner.calls.borrow();
    assert!(
        !calls.iter().any(|c| c.starts_with("shell:")),
        "shell must not run when override is set: {calls:?}"
    );
}

#[test]
fn setting_wins_over_login_shell() {
    let dir = tempfile::tempdir().expect("test fixture operation must succeed");
    let setting_bin = write_exec(dir.path(), "omp-setting");
    let runner = FakeRunner::default().with_version(&setting_bin, ok("omp/17.2.10"));

    let inputs = DiscoveryInputs {
        override_bin: None,
        setting_bin: Some(setting_bin.clone()),
        login_shell: Some(PathBuf::from("/bin/sh")),
        current_env: BTreeMap::new(),
    };

    let found = discover(&inputs, &runner).expect("test fixture operation must succeed");
    assert_eq!(found.path, setting_bin);
    assert!(
        !runner
            .calls
            .borrow()
            .iter()
            .any(|c| c.starts_with("shell:"))
    );
}

#[test]
fn relative_override_rejected() {
    let runner = FakeRunner::default();
    let inputs = DiscoveryInputs {
        override_bin: Some(PathBuf::from("relative/omp")),
        ..Default::default()
    };
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    let msg = format!("{err}");
    assert!(msg.contains("absolute"), "unexpected message: {msg}");
}

#[test]
fn relative_setting_rejected() {
    let runner = FakeRunner::default();
    let inputs = DiscoveryInputs {
        setting_bin: Some(PathBuf::from("relative/omp")),
        ..Default::default()
    };
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    assert!(format!("{err}").contains("absolute"));
}

#[test]
fn missing_override_binary() {
    let runner = FakeRunner::default();
    let inputs = DiscoveryInputs {
        override_bin: Some(PathBuf::from("/nonexistent/definitely/not/here/omp")),
        ..Default::default()
    };
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    assert!(matches!(err, RpcError::Discovery { .. }));
}

// ---------- login-shell env capture ----------------------------------

fn env_shell_stdout(binary_line: &str, entries: &[(&str, &str)], nul: bool) -> Vec<u8> {
    let marker = "--PIMIENTO-OMP-ENV-8f3c1a94b06d4e2a-BOUNDARY--";
    let mut out = Vec::new();
    out.extend_from_slice(binary_line.as_bytes());
    out.push(b'\n');
    out.push(b'\n'); // printf '\n%s\n' marker => leading blank line
    out.extend_from_slice(marker.as_bytes());
    out.push(b'\n');
    let sep = if nul { b'\0' } else { b'\n' };
    for (k, v) in entries {
        out.extend_from_slice(k.as_bytes());
        out.push(b'=');
        out.extend_from_slice(v.as_bytes());
        out.push(sep);
    }
    out
}

#[test]
fn shell_capture_nul_separated_env() {
    let dir = tempfile::tempdir().expect("test fixture operation must succeed");
    let bin = write_exec(dir.path(), "omp-in-path");
    let bin_str = bin.to_str().expect("test fixture operation must succeed");

    let stdout = env_shell_stdout(
        bin_str,
        &[
            ("PATH", "/usr/local/bin:/usr/bin"),
            ("HOME", "/Users/tester"),
            ("MULTI", "line\nvalue"), // NUL-safety: newline inside value
        ],
        true,
    );

    let runner = FakeRunner {
        shell_out: Some(CapturedOutput {
            status_success: true,
            exit_code: Some(0),
            stdout,
            stderr: Vec::new(),
        }),
        ..Default::default()
    }
    .with_version(&bin, ok("omp/17.2.10"));

    let inputs = DiscoveryInputs {
        login_shell: Some(PathBuf::from("/bin/zsh")),
        ..Default::default()
    };

    let found = discover(&inputs, &runner).expect("test fixture operation must succeed");
    assert_eq!(found.path, bin);
    assert_eq!(
        found
            .env
            .get(OsStr::new("PATH"))
            .map(|s| s.to_str().expect("PATH fixture value must be UTF-8")),
        Some("/usr/local/bin:/usr/bin"),
    );
    assert_eq!(
        found
            .env
            .get(OsStr::new("HOME"))
            .map(|s| s.to_str().expect("HOME fixture value must be UTF-8")),
        Some("/Users/tester"),
    );
    assert_eq!(
        found
            .env
            .get(OsStr::new("MULTI"))
            .map(|s| s.to_str().expect("MULTI fixture value must be UTF-8")),
        Some("line\nvalue"),
        "NUL-separated env must preserve embedded newlines",
    );
}

#[test]
fn shell_capture_newline_fallback_env() {
    let dir = tempfile::tempdir().expect("test fixture operation must succeed");
    let bin = write_exec(dir.path(), "omp");
    let bin_str = bin.to_str().expect("test fixture operation must succeed");

    let stdout = env_shell_stdout(
        bin_str,
        &[
            ("PATH", "/opt/homebrew/bin:/usr/bin"),
            ("SHELL", "/bin/bash"),
        ],
        false,
    );

    let runner = FakeRunner {
        shell_out: Some(CapturedOutput {
            status_success: true,
            exit_code: Some(0),
            stdout,
            stderr: Vec::new(),
        }),
        ..Default::default()
    }
    .with_version(&bin, ok("omp/17.2.10"));

    let inputs = DiscoveryInputs {
        login_shell: Some(PathBuf::from("/bin/bash")),
        ..Default::default()
    };

    let found = discover(&inputs, &runner).expect("test fixture operation must succeed");
    assert_eq!(
        found
            .env
            .get(OsStr::new("PATH"))
            .map(|s| s.to_str().expect("PATH fixture value must be UTF-8")),
        Some("/opt/homebrew/bin:/usr/bin"),
    );
    assert_eq!(
        found
            .env
            .get(OsStr::new("SHELL"))
            .map(|s| s.to_str().expect("SHELL fixture value must be UTF-8")),
        Some("/bin/bash"),
    );
}

#[test]
fn shell_capture_omp_not_found() {
    // `command -v` produced nothing before the marker.
    let stdout = env_shell_stdout("", &[("PATH", "/usr/bin")], true);
    let runner = FakeRunner {
        shell_out: Some(CapturedOutput {
            status_success: false,
            exit_code: Some(1),
            stdout,
            stderr: Vec::new(),
        }),
        ..Default::default()
    };
    let inputs = DiscoveryInputs {
        login_shell: Some(PathBuf::from("/bin/sh")),
        ..Default::default()
    };
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    match err {
        RpcError::Discovery { detail } => assert!(detail.contains("not found"), "got: {detail}"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn shell_capture_missing_marker_is_protocol_error() {
    let runner = FakeRunner {
        shell_out: Some(CapturedOutput {
            status_success: true,
            exit_code: Some(0),
            stdout: b"/usr/local/bin/omp\nPATH=/usr/bin\n".to_vec(),
            stderr: b"zsh: broken profile".to_vec(),
        }),
        ..Default::default()
    };
    let inputs = DiscoveryInputs {
        login_shell: Some(PathBuf::from("/bin/sh")),
        ..Default::default()
    };
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    let msg = format!("{err}");
    assert!(msg.contains("marker"), "unexpected message: {msg}");
}

// ---------- version probe --------------------------------------------

#[test]
fn version_probe_success_retains_verbatim_text() {
    let dir = tempfile::tempdir().expect("test fixture operation must succeed");
    let bin = write_exec(dir.path(), "omp");
    let banner = "omp/17.2.10\nrev abc123\n";
    let runner = FakeRunner::default().with_version(&bin, ok(banner));

    let inputs = DiscoveryInputs {
        override_bin: Some(bin.clone()),
        ..Default::default()
    };
    let found = discover(&inputs, &runner).expect("test fixture operation must succeed");
    assert_eq!(found.version_text, banner.trim());
    assert_eq!(found.version, MIN_SUPPORTED);
}

#[test]
fn version_probe_failure_surfaces_stderr() {
    let dir = tempfile::tempdir().expect("test fixture operation must succeed");
    let bin = write_exec(dir.path(), "omp");
    let runner = FakeRunner::default().with_version(
        &bin,
        CapturedOutput {
            status_success: false,
            exit_code: Some(2),
            stdout: Vec::new(),
            stderr: b"omp: license missing".to_vec(),
        },
    );

    let inputs = DiscoveryInputs {
        override_bin: Some(bin.clone()),
        ..Default::default()
    };
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    let msg = format!("{err}");
    assert!(msg.contains("license missing"), "got: {msg}");
    assert!(msg.contains("--version"));
}

// ---------- absent shell branch --------------------------------------

#[test]
fn no_shell_and_no_override_is_error() {
    let runner = FakeRunner::default();
    let inputs = DiscoveryInputs::default();
    let err = discover(&inputs, &runner).expect_err("test fixture operation must fail");
    assert!(matches!(err, RpcError::Discovery { .. }));
}
