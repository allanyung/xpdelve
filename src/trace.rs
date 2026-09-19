use std::io;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::config::TraceConfig;

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("resource not found")]
    ResourceNotFound { diagnostic: String },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Clone, Debug)]
pub struct TraceRequest {
    pub resource: String,
    pub context: Option<String>,
    pub namespace: Option<String>,
    pub kubeconfig: Option<PathBuf>,
    pub timeout: Option<Duration>,
}

#[derive(Debug)]
pub struct TraceOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub async fn execute(
    config: &TraceConfig,
    request: &TraceRequest,
    cancelled: CancellationToken,
) -> Result<TraceOutput, TraceError> {
    let (program, args) = resolved_command(config, request)?;
    let mut command = Command::new(&program);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    if let Some(kubeconfig) = &request.kubeconfig {
        command.env("KUBECONFIG", kubeconfig);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start trace command {program:?}"))?;
    let process_group = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .map(Pid::from_raw);
    let output_limit_reached = CancellationToken::new();
    let stdout = spawn_reader(
        child
            .stdout
            .take()
            .context("trace stdout was not captured")?,
        config.stdout_limit_bytes,
        "stdout",
        output_limit_reached.clone(),
    );
    let stderr = spawn_reader(
        child
            .stderr
            .take()
            .context("trace stderr was not captured")?,
        config.stderr_limit_bytes,
        "stderr",
        output_limit_reached.clone(),
    );

    let outcome = {
        let wait = async {
            match request.timeout {
                Some(timeout) => time::timeout(timeout, child.wait())
                    .await
                    .map_err(|_| anyhow!("trace command timed out after {}s", timeout.as_secs()))?
                    .context("failed while waiting for trace command"),
                None => child
                    .wait()
                    .await
                    .context("failed while waiting for trace command"),
            }
        };
        tokio::pin!(wait);
        tokio::select! {
            result = &mut wait => WaitOutcome::Completed(result),
            () = cancelled.cancelled() => WaitOutcome::Cancelled,
            () = output_limit_reached.cancelled() => WaitOutcome::OutputLimit,
        }
    };
    let status = match outcome {
        WaitOutcome::Completed(Ok(status)) => status,
        WaitOutcome::Completed(Err(error)) => {
            terminate_and_reap(&mut child, process_group).await;
            return Err(error.into());
        }
        WaitOutcome::Cancelled => {
            terminate_and_reap(&mut child, process_group).await;
            return Err(anyhow!("trace refresh cancelled").into());
        }
        WaitOutcome::OutputLimit => {
            terminate_and_reap(&mut child, process_group).await;
            let stdout_result = join_reader(stdout).await;
            let stderr_result = join_reader(stderr).await;
            return match (stdout_result, stderr_result) {
                (Err(error), _) | (_, Err(error)) => Err(error.into()),
                (Ok(_), Ok(_)) => Err(anyhow!("trace output limit exceeded").into()),
            };
        }
    };
    let stdout = join_reader(stdout).await?;
    let stderr = join_reader(stderr).await?;
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&stderr);
        if is_requested_resource_not_found(&diagnostic) {
            return Err(TraceError::ResourceNotFound {
                diagnostic: diagnostic.trim().to_owned(),
            });
        }
        return Err(TraceError::Other(anyhow!(
            "trace command exited with {status}: {}",
            diagnostic.trim()
        )));
    }
    Ok(TraceOutput { stdout, stderr })
}

fn is_requested_resource_not_found(diagnostic: &str) -> bool {
    let diagnostic = diagnostic.to_ascii_lowercase();
    diagnostic.contains("cannot get requested resource")
        && (diagnostic.contains("not found") || diagnostic.contains("does not exist"))
}

enum WaitOutcome {
    Completed(Result<ExitStatus>),
    Cancelled,
    OutputLimit,
}

pub fn resolved_command(
    config: &TraceConfig,
    request: &TraceRequest,
) -> Result<(String, Vec<String>)> {
    let mut args = config.args.clone();
    args.extend(replace(
        &config.resource_args,
        "{resource}",
        &request.resource,
    ));
    if let Some(context) = &request.context {
        args.extend(replace(&config.context_args, "{context}", context));
    }
    if let Some(namespace) = &request.namespace {
        args.extend(replace(&config.namespace_args, "{namespace}", namespace));
    }
    Ok((config.program.clone(), args))
}

fn replace(args: &[String], placeholder: &str, value: &str) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if arg == placeholder {
                value.to_owned()
            } else {
                arg.clone()
            }
        })
        .collect()
}

fn spawn_reader(
    reader: impl AsyncRead + Unpin + Send + 'static,
    limit: usize,
    stream: &'static str,
    output_limit_reached: CancellationToken,
) -> JoinHandle<Result<Vec<u8>>> {
    tokio::spawn(async move {
        let result = read_bounded(reader, limit, stream).await;
        if result.is_err() {
            output_limit_reached.cancel();
        }
        result
    })
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
    stream: &'static str,
) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if output.len().saturating_add(read) > limit {
            bail!("trace {stream} exceeded its {limit} byte limit");
        }
        output.extend_from_slice(&buffer[..read]);
    }
    Ok(output)
}

async fn join_reader(reader: JoinHandle<Result<Vec<u8>>>) -> Result<Vec<u8>> {
    reader
        .await
        .context("trace output reader stopped unexpectedly")?
}

async fn terminate_group(process_group: Option<Pid>) {
    let Some(process_group) = process_group else {
        return;
    };
    let _ = killpg(process_group, Signal::SIGTERM);
    time::sleep(Duration::from_millis(750)).await;
    let _ = killpg(process_group, Signal::SIGKILL);
}

async fn terminate_and_reap(child: &mut tokio::process::Child, process_group: Option<Pid>) {
    terminate_group(process_group).await;
    let _ = child.wait().await;
}

pub fn executable_available(program: &str) -> io::Result<bool> {
    if program.contains('/') {
        return executable_file(std::path::Path::new(program));
    }
    let Some(path) = std::env::var_os("PATH") else {
        return Ok(false);
    };
    Ok(std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .any(|candidate| executable_file(&candidate).unwrap_or(false)))
}

#[cfg(unix)]
fn executable_file(path: &std::path::Path) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)?;
    Ok(metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> TraceRequest {
        TraceRequest {
            resource: "Bucket/example".into(),
            context: None,
            namespace: None,
            kubeconfig: None,
            timeout: None,
        }
    }

    #[test]
    fn resolves_optional_arguments_without_dangling_flags() {
        let request = TraceRequest {
            namespace: Some("default".into()),
            ..request()
        };
        let (_, args) = resolved_command(&TraceConfig::default(), &request).unwrap();
        assert_eq!(
            args,
            [
                "resource",
                "trace",
                "-o",
                "json",
                "Bucket/example",
                "--namespace",
                "default"
            ]
        );
    }

    #[tokio::test]
    async fn captures_stdout_and_stderr_separately() {
        let config = TraceConfig {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf '{\"object\":{}}'; printf warning >&2".into(),
            ],
            resource_args: vec!["{resource}".into()],
            ..TraceConfig::default()
        };
        let output = execute(&config, &request(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(output.stdout, br#"{"object":{}}"#);
        assert_eq!(output.stderr, b"warning");
    }

    #[tokio::test]
    async fn rejects_output_over_the_configured_limit() {
        let config = TraceConfig {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "printf 12345".into()],
            resource_args: vec!["{resource}".into()],
            stdout_limit_bytes: 4,
            ..TraceConfig::default()
        };
        let error = execute(&config, &request(), CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stdout exceeded"));
    }

    #[tokio::test]
    async fn cancellation_stops_a_running_process() {
        let config = TraceConfig {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            resource_args: vec!["{resource}".into()],
            ..TraceConfig::default()
        };
        let token = CancellationToken::new();
        let cancel = token.clone();
        let task = tokio::spawn(async move { execute(&config, &request(), token).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        let result = time::timeout(Duration::from_secs(2), task)
            .await
            .expect("cancelled trace should complete")
            .unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
    }

    #[test]
    fn classifies_crossplane_requested_resource_failure_as_not_found() {
        assert!(is_requested_resource_not_found(
            r#"crossplane: error: cannot get requested resource: kind=Widget name=gone namespace=default: widgets.example.io "gone" not found"#
        ));
    }

    #[test]
    fn does_not_classify_child_not_found_as_missing_root() {
        assert!(!is_requested_resource_not_found(
            r#"configmaps "child" not found"#
        ));
    }
}
