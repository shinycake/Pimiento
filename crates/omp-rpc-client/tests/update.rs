//! OMP update parsing and command-invocation tests.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use omp_rpc_client::discovery::{CapturedOutput, CommandRunner};
use omp_rpc_client::{
    OmpUpdateCheck, OmpUpdateInstall, RpcError, check_omp_update, install_omp_update,
    parse_update_check_output,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct OmpCall {
    program: PathBuf,
    args: Vec<String>,
    env: BTreeMap<OsString, OsString>,
}

#[derive(Default)]
struct FakeRunner {
    outputs: RefCell<BTreeMap<Vec<String>, VecDeque<CapturedOutput>>>,
    failures: RefCell<BTreeMap<Vec<String>, String>>,
    calls: RefCell<Vec<OmpCall>>,
}

impl FakeRunner {
    fn with_output(self, args: &[&str], output: CapturedOutput) -> Self {
        self.outputs
            .borrow_mut()
            .entry(owned_args(args))
            .or_default()
            .push_back(output);
        self
    }

    fn with_failure(self, args: &[&str], detail: &str) -> Self {
        self.failures
            .borrow_mut()
            .insert(owned_args(args), detail.to_owned());
        self
    }
}

impl CommandRunner for FakeRunner {
    fn run_login_shell(&self, _shell: &OsStr, _script: &str) -> Result<CapturedOutput, RpcError> {
        Err(RpcError::Discovery {
            detail: "test: login shell must not run".to_owned(),
        })
    }

    fn run_omp(
        &self,
        program: &Path,
        args: &[&str],
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CapturedOutput, RpcError> {
        let args = owned_args(args);
        self.calls.borrow_mut().push(OmpCall {
            program: program.to_path_buf(),
            args: args.clone(),
            env: env.clone(),
        });

        if let Some(detail) = self.failures.borrow().get(&args) {
            return Err(RpcError::Discovery {
                detail: detail.clone(),
            });
        }

        self.outputs
            .borrow_mut()
            .get_mut(&args)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| RpcError::Discovery {
                detail: format!("test: no output configured for {args:?}"),
            })
    }
}

fn owned_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn output(success: bool, stdout: &str, stderr: &str) -> CapturedOutput {
    CapturedOutput {
        status_success: success,
        exit_code: Some(i32::from(!success)),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[test]
fn parses_plain_up_to_date_output() {
    let result =
        parse_update_check_output("Current version: 17.2.12\n✔ Already up to date\n", "", true);

    assert_eq!(
        result,
        OmpUpdateCheck::UpToDate {
            current: "17.2.12".to_owned(),
        }
    );
}

#[test]
fn parses_ansi_up_to_date_output_without_current_version() {
    let result = parse_update_check_output("\u{1b}[32m✔ Already up to date\u{1b}[0m\n", "", true);

    assert_eq!(
        result,
        OmpUpdateCheck::UpToDate {
            current: "unknown".to_owned(),
        }
    );
}

#[test]
fn parses_plain_available_output() {
    let result = parse_update_check_output(
        "Current version: 17.2.11\nNew version available: 17.2.12\n",
        "",
        true,
    );

    assert_eq!(
        result,
        OmpUpdateCheck::Available {
            current: "17.2.11".to_owned(),
            latest: "17.2.12".to_owned(),
        }
    );
}

#[test]
fn parses_ansi_available_output() {
    let result = parse_update_check_output(
        "\u{1b}[36mCurrent version: \u{1b}[0m\u{1b}[32m17.2.11\u{1b}[0m\n\
         \u{1b}[36mNew version available: \u{1b}[33m17.2.12\u{1b}[0m\n",
        "",
        true,
    );

    assert_eq!(
        result,
        OmpUpdateCheck::Available {
            current: "17.2.11".to_owned(),
            latest: "17.2.12".to_owned(),
        }
    );
}

#[test]
fn check_failure_includes_clean_stdout_and_stderr() {
    let result = parse_update_check_output(
        "\u{1b}[31mFailed to check for updates: registry unavailable\u{1b}[0m",
        "connection reset",
        false,
    );

    let OmpUpdateCheck::Failed { detail } = result else {
        panic!("expected failed update check");
    };
    assert!(detail.contains("Failed to check for updates"));
    assert!(detail.contains("connection reset"));
    assert!(!detail.contains("\u{1b}["));
}

#[test]
fn unrecognized_success_output_is_a_failed_check() {
    let result = parse_update_check_output("unexpected output", "", true);

    let OmpUpdateCheck::Failed { detail } = result else {
        panic!("expected failed update check");
    };
    assert!(detail.contains("could not parse"));
    assert!(detail.contains("unexpected output"));
}

#[test]
fn check_invokes_update_check_with_supplied_environment() {
    let runner = FakeRunner::default().with_output(
        &["update", "--check"],
        output(
            true,
            "Current version: 17.2.11\nNew version available: 17.2.12\n",
            "",
        ),
    );
    let program = Path::new("/home/test/.local/bin/omp");
    let env = BTreeMap::from([(
        OsString::from("PATH"),
        OsString::from("/home/test/.local/bin:/usr/bin"),
    )]);

    let result = check_omp_update(program, &env, &runner);

    assert!(matches!(result, OmpUpdateCheck::Available { .. }));
    assert_eq!(
        runner.calls.borrow().as_slice(),
        &[OmpCall {
            program: program.to_path_buf(),
            args: owned_args(&["update", "--check"]),
            env,
        }]
    );
}

#[test]
fn check_transport_failure_is_returned_as_a_value() {
    let runner = FakeRunner::default().with_failure(&["update", "--check"], "test spawn failure");

    let result = check_omp_update(Path::new("/missing/omp"), &BTreeMap::new(), &runner);

    let OmpUpdateCheck::Failed { detail } = result else {
        panic!("expected failed update check");
    };
    assert!(detail.contains("test spawn failure"));
}

#[test]
fn install_invokes_update_then_reprobes_replaced_binary() {
    let runner = FakeRunner::default()
        .with_output(
            &["update"],
            output(true, "Updated to 17.2.12\n", "installer notice"),
        )
        .with_output(&["--version"], output(true, "omp/17.2.12\n", ""));
    let program = Path::new("/home/test/.local/bin/omp");
    let env = BTreeMap::from([(OsString::from("HOME"), OsString::from("/home/test"))]);

    let result = install_omp_update(program, &env, &runner);

    assert_eq!(
        result,
        OmpUpdateInstall::Updated {
            previous: None,
            current: "17.2.12".to_owned(),
            raw: "Updated to 17.2.12\ninstaller notice".to_owned(),
        }
    );
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].args, owned_args(&["update"]));
    assert_eq!(calls[1].args, owned_args(&["--version"]));
    assert!(calls.iter().all(|call| call.program == program));
    assert!(calls.iter().all(|call| call.env == env));
}

#[test]
fn install_failure_does_not_reprobe() {
    let runner = FakeRunner::default().with_output(
        &["update"],
        output(
            false,
            "",
            "\u{1b}[31mUpdate failed: permission denied\u{1b}[0m",
        ),
    );

    let result = install_omp_update(
        Path::new("/home/test/.local/bin/omp"),
        &BTreeMap::new(),
        &runner,
    );

    let OmpUpdateInstall::Failed { detail, raw } = result else {
        panic!("expected failed update install");
    };
    assert!(detail.contains("Update failed: permission denied"));
    assert!(!detail.contains("\u{1b}["));
    assert!(raw.contains("\u{1b}[31m"));
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].args, owned_args(&["update"]));
}

#[test]
fn successful_install_uses_reported_version_when_reprobe_fails() {
    let runner = FakeRunner::default()
        .with_output(
            &["update"],
            output(true, "Current version: 17.2.11\nUpdated to 17.2.12\n", ""),
        )
        .with_failure(&["--version"], "binary temporarily unavailable");

    let result = install_omp_update(
        Path::new("/home/test/.local/bin/omp"),
        &BTreeMap::new(),
        &runner,
    );

    assert_eq!(
        result,
        OmpUpdateInstall::Updated {
            previous: Some("17.2.11".to_owned()),
            current: "17.2.12".to_owned(),
            raw: "Current version: 17.2.11\nUpdated to 17.2.12".to_owned(),
        }
    );
}
