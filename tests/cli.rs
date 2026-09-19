use std::process::Command;

fn xpdelve() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xpdelve"))
}

#[test]
fn version_reports_project_version() {
    let output = xpdelve().arg("version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("app: xpdelve"));
    assert!(stdout.contains("version: 0.1.0"));
}

#[test]
fn no_arguments_prints_help() {
    let output = xpdelve().output().unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Usage: xpdelve")
    );
}

#[test]
fn live_trace_requires_a_terminal() {
    let output = xpdelve().arg("Bucket/example").output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("requires an interactive terminal")
    );
}
