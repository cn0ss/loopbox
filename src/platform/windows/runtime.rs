use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub fn supports_process_groups() -> bool {
    false
}

pub fn configure_process_group(_command: &mut Command) {
    // No-op on Windows — no process groups
}

pub fn run_pty_child(
    _command_str: &str,
    _workdir: &str,
    _log_file_path: &Path,
    _input_path: &Path,
) -> Result<i32, String> {
    Err("PTY mode is not supported on Windows. Use standard process mode.".to_string())
}

pub fn spawn_pty_child(
    _command_str: &str,
    _workdir: &str,
    _cols: u16,
    _rows: u16,
) -> Result<(i32, i32), String> {
    Err("PTY mode is not supported on Windows. Use standard process mode.".to_string())
}

pub fn close_fd(_fd: i32) {}

pub fn resize_pty(
    _fd: i32,
    _cols: u16,
    _rows: u16,
    _cell_width_px: u32,
    _cell_height_px: u32,
) -> Result<(), String> {
    Err("PTY resize is not supported on Windows.".to_string())
}

pub fn wait_for_child_exit(_pid: i32) -> Result<i32, String> {
    Err("wait_for_child_exit is not supported on Windows.".to_string())
}

pub struct RawStdinGuard;
impl Drop for RawStdinGuard {
    fn drop(&mut self) {}
}

pub fn set_stdin_raw_mode() -> Result<RawStdinGuard, String> {
    Err("Raw stdin mode is not supported on Windows.".to_string())
}

pub fn forward_terminal_input_to_fifo(
    _input_path: &Path,
    _stop: Arc<AtomicBool>,
) -> Result<(), String> {
    Err("FIFO terminal input is not supported on Windows.".to_string())
}

pub fn open_fifo_read_nonblocking(_path: &Path) -> Result<i32, String> {
    Err("FIFO is not supported on Windows.".to_string())
}

pub fn forward_fifo_input_to_pty(
    _input_path: std::path::PathBuf,
    _pty_writer_fd: i32,
    _stop: Arc<AtomicBool>,
) -> Result<(), String> {
    Err("PTY is not supported on Windows.".to_string())
}

pub fn write_all_fd(_fd: i32, _payload: &[u8]) -> Result<(), String> {
    Err("Raw fd write is not supported on Windows.".to_string())
}

pub fn write_pty_output_to_log(
    _pty_reader_fd: i32,
    _log_file: std::fs::File,
) -> Result<(), String> {
    Err("PTY log output is not supported on Windows.".to_string())
}

pub fn follow_log_output<F>(
    log_file: &Path,
    mut offset: u64,
    stop: Arc<AtomicBool>,
    file_length_fn: impl Fn(&Path) -> Result<u64, String>,
    read_delta_fn: F,
) -> Result<(), String>
where
    F: Fn(&Path, u64) -> Result<(Vec<u8>, u64), String>,
{
    use std::io::Write;
    use std::thread;
    use std::time::Duration;

    offset = offset.min(file_length_fn(log_file).unwrap_or(0));
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if log_file.exists() {
            match read_delta_fn(log_file, offset) {
                Ok((bytes, end_offset)) => {
                    offset = end_offset;
                    if !bytes.is_empty() {
                        let mut stdout = std::io::stdout().lock();
                        let _ = stdout.write_all(&bytes);
                        let _ = stdout.flush();
                    }
                }
                Err(err) => eprintln!("Loopbox runtime attach log warning: {err}"),
            }
        }
        thread::sleep(Duration::from_millis(120));
    }
    Ok(())
}

pub fn listening_pid_for_port(bind_ip: &str, port: u16) -> Option<u32> {
    // Use: netstat -ano | findstr ":{port}"
    let output = Command::new("cmd")
        .args(["/C", &format!("netstat -ano | findstr \":{port}\"")])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains(&format!("{bind_ip}:{port}")) && line.contains("LISTENING") {
            return line.split_whitespace().last()?.parse().ok();
        }
    }
    None
}

pub fn process_command_for_pid(pid: u32) -> Option<String> {
    // Use: wmic process where ProcessId={pid} get Name
    let output = Command::new("wmic")
        .args([
            "process",
            "where",
            &format!("ProcessId={pid}"),
            "get",
            "Name",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1)?
        .trim()
        .to_string()
        .into()
}

pub fn secure_file_permissions(_path: &Path) -> Result<(), String> {
    // Windows file ACLs are different; skip for now.
    // Could use icacls in the future.
    Ok(())
}
