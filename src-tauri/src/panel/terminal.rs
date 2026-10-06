use std::{path::Path, process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt, process::Command};

use crate::ai::tools::resolve_within;

const MAX_OUTPUT_BYTES: usize = 256 * 1024;

#[tauri::command]
pub async fn panel_terminal_run(command: String, workspace: String) -> Result<String, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Enter a command".into());
    }
    let root = Path::new(&workspace)
        .canonicalize()
        .map_err(|error| format!("Could not open workspace: {error}"))?;
    let working_directory = resolve_within(&root, ".")?;

    let mut child = if cfg!(windows) {
        Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "$global:LASTEXITCODE = $null; {command}; $commandSucceeded = $?; if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }}; if ($commandSucceeded) {{ exit 0 }} else {{ exit 1 }}"
            ))
            .current_dir(&working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("Could not start PowerShell: {error}"))?
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("Could not start shell: {error}"))?
    };

    let stdout = child.stdout.take().ok_or("Could not capture stdout")?;
    let stderr = child.stderr.take().ok_or("Could not capture stderr")?;
    let collect = async {
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let mut stdout = stdout.take(MAX_OUTPUT_BYTES as u64);
        let mut stderr = stderr.take(MAX_OUTPUT_BYTES as u64);
        let stdout_read = stdout.read_to_end(&mut stdout_bytes);
        let stderr_read = stderr.read_to_end(&mut stderr_bytes);
        let (stdout_result, stderr_result) = tokio::join!(stdout_read, stderr_read);
        stdout_result.map_err(|error| format!("Could not read stdout: {error}"))?;
        stderr_result.map_err(|error| format!("Could not read stderr: {error}"))?;
        let mut output = stdout_bytes;
        if !output.is_empty() && !stderr_bytes.is_empty() {
            output.push(b'\n');
        }
        output.extend(stderr_bytes);
        let status = child
            .wait()
            .await
            .map_err(|error| format!("Could not wait for command: {error}"))?;
        Ok::<_, String>((output, status))
    };
    let (output, status) = tokio::time::timeout(Duration::from_secs(120), collect)
        .await
        .map_err(|_| "Command timed out after 120 seconds".to_string())??;
    let truncated = output.len() >= MAX_OUTPUT_BYTES;
    let output = String::from_utf8_lossy(&output);
    let suffix = if truncated {
        "\n[Output truncated]"
    } else {
        ""
    };
    Ok(format!(
        "Exit code: {}\n{}{}",
        status.code().unwrap_or(-1),
        output,
        suffix
    ))
}
