//! Unix-specific runtime helpers extracted from `loopbox::runtime`.
//!
//! Every public function here is called from the business-logic layer in
//! `crate::loopbox::runtime`.  The module is only compiled on `cfg(unix)` /
//! `cfg(target_os = "macos")` via the parent platform gate.

use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rustix::fs::{self as rustix_fs, Mode, OFlags};
use rustix::io::Errno;
use rustix::process::{self as rustix_process, Pid, Signal, WaitOptions, WaitStatus};
use rustix::termios::{self as rustix_termios, OptionalActions, Termios, Winsize};

pub type RuntimeFd = OwnedFd;
pub type RuntimePid = libc::pid_t;

// ---------------------------------------------------------------------------
// Process group
// ---------------------------------------------------------------------------

/// Whether this platform supports process groups for service isolation.
pub fn supports_process_groups() -> bool {
    true
}

/// Place a `Command` into its own process group so that stop/restart can
/// terminate the full subtree without touching loopbox itself.
pub fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

// ---------------------------------------------------------------------------
// PTY runner  (the big forkpty block)
// ---------------------------------------------------------------------------

/// Fork a PTY child that runs `command` (via `/bin/bash -lc`) inside `workdir`,
/// returning the PTY master file descriptor and child PID.
pub fn spawn_pty_child(
    command_str: &str,
    workdir: &str,
    cols: u16,
    rows: u16,
) -> Result<(RuntimeFd, RuntimePid), String> {
    let shell = CString::new("/bin/bash").expect("static path without NUL");
    let arg_lc = CString::new("-lc").expect("static arg without NUL");
    let command_arg = CString::new(command_str)
        .map_err(|_| "Runtime PTY command contains an unsupported NUL byte.".to_string())?;
    let workdir_c = CString::new(workdir)
        .map_err(|_| "Runtime PTY workdir contains an unsupported NUL byte.".to_string())?;

    let mut master_fd: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let child_pid = unsafe {
        libc::forkpty(
            &mut master_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize,
        )
    };
    if child_pid < 0 {
        return Err(format!(
            "Failed to create runtime PTY for '{}': {}",
            command_str,
            std::io::Error::last_os_error()
        ));
    }

    if child_pid == 0 {
        unsafe {
            if libc::chdir(workdir_c.as_ptr()) != 0 {
                libc::_exit(127);
            }
            let argv = [
                shell.as_ptr(),
                arg_lc.as_ptr(),
                command_arg.as_ptr(),
                std::ptr::null(),
            ];
            libc::execv(shell.as_ptr(), argv.as_ptr());
            libc::_exit(127);
        }
    }

    // SAFETY: `forkpty` initialized `master_fd` with an owned descriptor in
    // the parent process when it returned a positive child pid.
    let master_fd = unsafe { OwnedFd::from_raw_fd(master_fd) };
    Ok((master_fd, child_pid))
}

pub fn resize_pty(
    fd: &RuntimeFd,
    cols: u16,
    rows: u16,
    cell_width_px: u32,
    cell_height_px: u32,
) -> Result<(), String> {
    let winsize = Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: cell_width_px
            .saturating_mul(u32::from(cols))
            .min(u32::from(u16::MAX)) as u16,
        ws_ypixel: cell_height_px
            .saturating_mul(u32::from(rows))
            .min(u32::from(u16::MAX)) as u16,
    };
    rustix_termios::tcsetwinsize(fd, winsize).map_err(|err| {
        format!(
            "Failed to resize runtime PTY to {cols}x{rows}: {}",
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })
}

/// Fork a PTY child that runs `command` (via `/bin/bash -lc`) inside `workdir`,
/// logging all output to `log_file` and accepting input from the FIFO at
/// `input_path`.  Returns the child exit code.
pub fn run_pty_child(
    command_str: &str,
    workdir: &str,
    log_file_path: &Path,
    input_path: &Path,
) -> Result<i32, String> {
    if let Some(parent) = log_file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path)
        .map_err(|err| {
            format!(
                "Failed to open runtime PTY log file '{}': {err}",
                log_file_path.display()
            )
        })?;

    let (master_fd, child_pid) = spawn_pty_child(command_str, workdir, 80, 24)?;
    let writer_fd = match rustix::io::dup(&master_fd) {
        Ok(fd) => fd,
        Err(err) => {
            drop(master_fd);
            terminate_process(child_pid, Signal::TERM);
            let _ = wait_for_child_exit(child_pid);
            return Err(format!(
                "Failed to duplicate runtime PTY descriptor: {}",
                std::io::Error::from_raw_os_error(err.raw_os_error())
            ));
        }
    };

    let stop = Arc::new(AtomicBool::new(false));
    let input_stop = Arc::clone(&stop);
    let input_path_owned = input_path.to_path_buf();
    let input_thread =
        thread::spawn(move || forward_fifo_input_to_pty(input_path_owned, writer_fd, input_stop));

    let output_thread = thread::spawn(move || write_pty_output_to_log(master_fd, log_file));

    let exit_code = wait_for_child_exit(child_pid)?;
    stop.store(true, Ordering::SeqCst);

    match input_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("Loopbox runtime PTY input warning: {err}"),
        Err(err) => eprintln!("Loopbox runtime PTY input thread panicked: {err:?}"),
    }
    match output_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("Loopbox runtime PTY output warning: {err}"),
        Err(err) => eprintln!("Loopbox runtime PTY output thread panicked: {err:?}"),
    }

    Ok(exit_code)
}

// ---------------------------------------------------------------------------
// wait_for_child_exit
// ---------------------------------------------------------------------------

pub fn wait_for_child_exit(pid: RuntimePid) -> Result<i32, String> {
    let pid = runtime_pid(pid)?;
    loop {
        match rustix_process::waitpid(Some(pid), WaitOptions::empty()) {
            Ok(Some((_pid, status))) => return Ok(wait_status_exit_code(status)),
            Ok(None) => continue,
            Err(Errno::INTR) => continue,
            Err(err) => {
                let io_err = std::io::Error::from_raw_os_error(err.raw_os_error());
                return Err(format!(
                    "Failed while waiting for runtime PTY child {}: {io_err}",
                    pid.as_raw_pid()
                ));
            }
        }
    }
}

pub fn try_wait_child_exit(pid: RuntimePid) -> Result<Option<i32>, String> {
    let pid = runtime_pid(pid)?;
    match rustix_process::waitpid(Some(pid), WaitOptions::NOHANG) {
        Ok(Some((_pid, status))) => Ok(Some(wait_status_exit_code(status))),
        Ok(None) => Ok(None),
        Err(Errno::CHILD) => Ok(Some(0)),
        Err(err) => {
            let io_err = std::io::Error::from_raw_os_error(err.raw_os_error());
            Err(format!(
                "Failed while waiting for runtime terminal child {}: {io_err}",
                pid.as_raw_pid()
            ))
        }
    }
}

pub fn terminate_process(pid: RuntimePid, signal: Signal) {
    if let Ok(pid) = runtime_pid(pid) {
        let _ = rustix_process::kill_process(pid, signal);
    }
}

pub fn set_fd_nonblocking(fd: &RuntimeFd) -> Result<(), String> {
    let flags = rustix_fs::fcntl_getfl(fd).map_err(|err| {
        format!(
            "Failed to read terminal fd flags: {}",
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })?;
    rustix_fs::fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(|err| {
        format!(
            "Failed to set terminal fd nonblocking: {}",
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })
}

pub enum ReadFdError {
    WouldBlock,
    Closed,
    Other(String),
}

pub fn read_fd(fd: &RuntimeFd, buffer: &mut [u8]) -> Result<usize, ReadFdError> {
    match rustix::io::read(fd, buffer) {
        Ok(read_count) => Ok(read_count),
        Err(Errno::INTR) | Err(Errno::AGAIN) => Err(ReadFdError::WouldBlock),
        Err(Errno::IO) | Err(Errno::BADF) => Err(ReadFdError::Closed),
        Err(err) => {
            let io_err = std::io::Error::from_raw_os_error(err.raw_os_error());
            Err(ReadFdError::Other(format!(
                "Failed to read runtime terminal PTY: {io_err}"
            )))
        }
    }
}

fn runtime_pid(pid: RuntimePid) -> Result<Pid, String> {
    Pid::from_raw(pid).ok_or_else(|| format!("Invalid runtime child pid {pid}."))
}

fn wait_status_exit_code(status: WaitStatus) -> i32 {
    if let Some(code) = status.exit_status() {
        return code;
    }
    if let Some(signal) = status.terminating_signal() {
        return 128 + signal;
    }
    1
}

// ---------------------------------------------------------------------------
// Terminal raw mode
// ---------------------------------------------------------------------------

pub struct RawStdinGuard {
    original: Termios,
}

impl Drop for RawStdinGuard {
    fn drop(&mut self) {
        let _ =
            rustix_termios::tcsetattr(rustix::stdio::stdin(), OptionalActions::Now, &self.original);
    }
}

pub fn set_stdin_raw_mode() -> Result<RawStdinGuard, String> {
    let stdin_fd = rustix::stdio::stdin();
    if !rustix_termios::isatty(stdin_fd) {
        return Err("Attach mode requires an interactive terminal (TTY stdin).".to_string());
    }

    let term = rustix_termios::tcgetattr(stdin_fd).map_err(|err| {
        format!(
            "Failed to read terminal mode for attach: {}",
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })?;
    let mut raw = term.clone();
    raw.make_raw();
    rustix_termios::tcsetattr(stdin_fd, OptionalActions::Now, &raw).map_err(|err| {
        format!(
            "Failed to set terminal raw mode for attach: {}",
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })?;
    Ok(RawStdinGuard { original: term })
}

// ---------------------------------------------------------------------------
// FIFO / FD operations
// ---------------------------------------------------------------------------

pub fn forward_terminal_input_to_fifo(
    input_path: &Path,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let fifo_fd = rustix_fs::open(input_path, OFlags::WRONLY, Mode::empty()).map_err(|err| {
        format!(
            "Failed to open runtime input fifo '{}': {}",
            input_path.display(),
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })?;

    let raw_guard = set_stdin_raw_mode()?;
    let stdin_fd = rustix::stdio::stdin();
    let mut buffer = [0_u8; 64];
    while !stop.load(Ordering::Relaxed) {
        let read_count = match rustix::io::read(stdin_fd, &mut buffer) {
            Ok(read_count) => read_count,
            Err(Errno::INTR) => continue,
            Err(err) => {
                drop(fifo_fd);
                drop(raw_guard);
                let io_err = std::io::Error::from_raw_os_error(err.raw_os_error());
                return Err(format!(
                    "Failed to read terminal input for attach mode: {io_err}"
                ));
            }
        };
        if read_count > 0 {
            let mut chunk = &buffer[..read_count];
            while !chunk.is_empty() {
                if let Some(index) = chunk.iter().position(|byte| *byte == 0x1d) {
                    if index > 0 {
                        write_all_fd(&fifo_fd, &chunk[..index])?;
                    }
                    drop(fifo_fd);
                    drop(raw_guard);
                    println!("\nDetached from service.");
                    let _ = std::io::stdout().flush();
                    return Ok(());
                }
                write_all_fd(&fifo_fd, chunk)?;
                chunk = &[];
            }
            continue;
        }
        if read_count == 0 {
            break;
        }
    }

    drop(fifo_fd);
    drop(raw_guard);
    Ok(())
}

pub fn open_fifo_read_nonblocking(path: &Path) -> Result<RuntimeFd, String> {
    rustix_fs::open(path, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty()).map_err(|err| {
        format!(
            "Failed to open runtime input fifo '{}': {}",
            path.display(),
            std::io::Error::from_raw_os_error(err.raw_os_error())
        )
    })
}

pub fn forward_fifo_input_to_pty(
    input_path: PathBuf,
    pty_writer_fd: RuntimeFd,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let fifo_fd = open_fifo_read_nonblocking(&input_path)?;
    let mut buffer = [0_u8; 512];
    while !stop.load(Ordering::Relaxed) {
        let read_bytes = match rustix::io::read(&fifo_fd, &mut buffer) {
            Ok(bytes) => bytes,
            Err(Errno::INTR) | Err(Errno::AGAIN) => {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(Errno::BADF) => break,
            Err(err) => {
                let io_err = std::io::Error::from_raw_os_error(err.raw_os_error());
                drop(fifo_fd);
                drop(pty_writer_fd);
                return Err(format!(
                    "Failed to read runtime input fifo '{}': {io_err}",
                    input_path.display()
                ));
            }
        };
        if read_bytes > 0 {
            write_all_fd(&pty_writer_fd, &buffer[..read_bytes])?;
            continue;
        }
        if read_bytes == 0 {
            thread::sleep(Duration::from_millis(30));
            continue;
        }
    }
    drop(fifo_fd);
    drop(pty_writer_fd);
    Ok(())
}

pub fn write_all_fd(fd: &RuntimeFd, payload: &[u8]) -> Result<(), String> {
    let mut offset = 0_usize;
    while offset < payload.len() {
        let write_result = match rustix::io::write(fd, &payload[offset..]) {
            Ok(bytes) => bytes,
            Err(Errno::INTR) => continue,
            Err(Errno::PIPE) | Err(Errno::IO) | Err(Errno::BADF) => return Ok(()),
            Err(err) => {
                let io_err = std::io::Error::from_raw_os_error(err.raw_os_error());
                return Err(format!("Failed to write runtime PTY input: {io_err}"));
            }
        };
        if write_result > 0 {
            offset += write_result;
            continue;
        }
        if write_result == 0 {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
    }
    Ok(())
}

pub fn write_pty_output_to_log(
    pty_reader_fd: RuntimeFd,
    mut log_file: fs::File,
) -> Result<(), String> {
    let mut reader = fs::File::from(pty_reader_fd);
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => {
                log_file
                    .write_all(&buffer[..bytes_read])
                    .map_err(|err| format!("Failed to write runtime PTY log output: {err}"))?;
            }
            Err(err) => match err.raw_os_error() {
                Some(code) if code == libc::EINTR => continue,
                Some(code) if code == libc::EIO || code == libc::EBADF => break,
                _ => return Err(format!("Failed to read runtime PTY output: {err}")),
            },
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Log following
// ---------------------------------------------------------------------------

/// Tail a log file, writing new content to stdout until `stop` is set.
/// `read_delta` is a callback that reads a byte range from the log file
/// (to avoid coupling this module to the log-management layer).
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
    offset = offset.min(file_length_fn(log_file).unwrap_or(0));
    while !stop.load(Ordering::Relaxed) {
        if log_file.exists() {
            match read_delta_fn(log_file, offset) {
                Ok((bytes, end_offset)) => {
                    offset = end_offset;
                    if !bytes.is_empty() {
                        let mut stdout = std::io::stdout().lock();
                        stdout
                            .write_all(&bytes)
                            .map_err(|err| format!("Failed to write attach output: {err}"))?;
                        stdout
                            .flush()
                            .map_err(|err| format!("Failed to flush attach output: {err}"))?;
                    }
                }
                Err(err) => eprintln!("Loopbox runtime attach log warning: {err}"),
            }
        }
        thread::sleep(Duration::from_millis(120));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Process inspection
// ---------------------------------------------------------------------------

pub fn listening_pid_for_port(bind_ip: &str, port: u16) -> Option<u32> {
    let target = format!("TCP@{}:{}", bind_ip.trim(), port);
    let output = Command::new("/usr/sbin/lsof")
        .arg("-n")
        .arg("-P")
        .arg("-t")
        .arg("-i")
        .arg(target)
        .arg("-sTCP:LISTEN")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok())
}

pub fn process_command_for_pid(pid: u32) -> Option<String> {
    let output = Command::new("/bin/ps")
        .arg("-o")
        .arg("comm=")
        .arg("-p")
        .arg(pid.to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

// ---------------------------------------------------------------------------
// File permissions  (from agent_api.rs)
// ---------------------------------------------------------------------------

/// Set file permissions to owner-only read/write (chmod 0o600).
pub fn secure_file_permissions(path: &Path) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("Failed to secure {} permissions: {err}", path.display()))
}
