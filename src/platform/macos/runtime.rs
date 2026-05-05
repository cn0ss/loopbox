//! Unix-specific runtime helpers extracted from `loopbox::runtime`.
//!
//! Every public function here is called from the business-logic layer in
//! `crate::loopbox::runtime`.  The module is only compiled on `cfg(unix)` /
//! `cfg(target_os = "macos")` via the parent platform gate.

use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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
) -> Result<(libc::c_int, libc::pid_t), String> {
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

    Ok((master_fd, child_pid))
}

pub fn close_fd(fd: libc::c_int) {
    unsafe {
        libc::close(fd);
    }
}

pub fn resize_pty(
    fd: libc::c_int,
    cols: u16,
    rows: u16,
    cell_width_px: u32,
    cell_height_px: u32,
) -> Result<(), String> {
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: cell_width_px
            .saturating_mul(u32::from(cols))
            .min(u32::from(u16::MAX)) as u16,
        ws_ypixel: cell_height_px
            .saturating_mul(u32::from(rows))
            .min(u32::from(u16::MAX)) as u16,
    };
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &mut winsize) } != 0 {
        return Err(format!(
            "Failed to resize runtime PTY to {cols}x{rows}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
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
    let writer_fd = unsafe { libc::dup(master_fd) };
    if writer_fd < 0 {
        unsafe {
            libc::close(master_fd);
            libc::kill(child_pid, libc::SIGTERM);
        }
        let _ = wait_for_child_exit(child_pid);
        return Err(format!(
            "Failed to duplicate runtime PTY descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }

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

pub fn wait_for_child_exit(pid: libc::pid_t) -> Result<i32, String> {
    let mut status = 0_i32;
    loop {
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        if waited == pid {
            let exited = libc::WIFEXITED(status);
            if exited {
                return Ok(libc::WEXITSTATUS(status) as i32);
            }
            let signaled = libc::WIFSIGNALED(status);
            if signaled {
                return Ok(128 + libc::WTERMSIG(status) as i32);
            }
            return Ok(1);
        }
        if waited < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(format!(
                "Failed while waiting for runtime PTY child {pid}: {err}"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal raw mode
// ---------------------------------------------------------------------------

pub struct RawStdinGuard {
    fd: libc::c_int,
    original: libc::termios,
}

impl Drop for RawStdinGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

pub fn set_stdin_raw_mode() -> Result<RawStdinGuard, String> {
    let stdin_fd = std::io::stdin().as_raw_fd();
    if unsafe { libc::isatty(stdin_fd) } != 1 {
        return Err("Attach mode requires an interactive terminal (TTY stdin).".to_string());
    }

    let mut term = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(stdin_fd, &mut term) } != 0 {
        return Err(format!(
            "Failed to read terminal mode for attach: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut raw = term;
    unsafe {
        libc::cfmakeraw(&mut raw);
    }
    if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
        return Err(format!(
            "Failed to set terminal raw mode for attach: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(RawStdinGuard {
        fd: stdin_fd,
        original: term,
    })
}

// ---------------------------------------------------------------------------
// FIFO / FD operations
// ---------------------------------------------------------------------------

pub fn forward_terminal_input_to_fifo(
    input_path: &Path,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let c_path = CString::new(input_path.to_string_lossy().into_owned()).map_err(|_| {
        format!(
            "Input fifo path contains unsupported NUL bytes: '{}'",
            input_path.display()
        )
    })?;
    let fifo_fd = unsafe { libc::open(c_path.as_ptr(), libc::O_WRONLY) };
    if fifo_fd < 0 {
        return Err(format!(
            "Failed to open runtime input fifo '{}': {}",
            input_path.display(),
            std::io::Error::last_os_error()
        ));
    }

    let raw_guard = set_stdin_raw_mode()?;
    let stdin_fd = std::io::stdin().as_raw_fd();
    let mut buffer = [0_u8; 64];
    while !stop.load(Ordering::Relaxed) {
        let read_count = unsafe {
            libc::read(
                stdin_fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            )
        };
        if read_count > 0 {
            let mut chunk = &buffer[..read_count as usize];
            while !chunk.is_empty() {
                if let Some(index) = chunk.iter().position(|byte| *byte == 0x1d) {
                    if index > 0 {
                        write_all_fd(fifo_fd, &chunk[..index])?;
                    }
                    unsafe {
                        libc::close(fifo_fd);
                    }
                    drop(raw_guard);
                    println!("\nDetached from service.");
                    let _ = std::io::stdout().flush();
                    return Ok(());
                }
                write_all_fd(fifo_fd, chunk)?;
                chunk = &[];
            }
            continue;
        }
        if read_count == 0 {
            break;
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EINTR => continue,
            _ => {
                unsafe {
                    libc::close(fifo_fd);
                }
                drop(raw_guard);
                return Err(format!(
                    "Failed to read terminal input for attach mode: {err}"
                ));
            }
        }
    }

    unsafe {
        libc::close(fifo_fd);
    }
    drop(raw_guard);
    Ok(())
}

pub fn open_fifo_read_nonblocking(path: &Path) -> Result<libc::c_int, String> {
    let raw = path.to_string_lossy().into_owned();
    let c_path = CString::new(raw.as_str())
        .map_err(|_| format!("Input fifo path contains unsupported NUL bytes: '{}'", raw))?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if fd < 0 {
        return Err(format!(
            "Failed to open runtime input fifo '{}': {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(fd)
}

pub fn forward_fifo_input_to_pty(
    input_path: PathBuf,
    pty_writer_fd: libc::c_int,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let fifo_fd = open_fifo_read_nonblocking(&input_path)?;
    let mut buffer = [0_u8; 512];
    while !stop.load(Ordering::Relaxed) {
        let read_bytes = unsafe {
            libc::read(
                fifo_fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            )
        };
        if read_bytes > 0 {
            write_all_fd(pty_writer_fd, &buffer[..read_bytes as usize])?;
            continue;
        }
        if read_bytes == 0 {
            thread::sleep(Duration::from_millis(30));
            continue;
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EINTR || code == libc::EAGAIN => {
                thread::sleep(Duration::from_millis(20));
            }
            Some(code) if code == libc::EBADF => break,
            _ => {
                unsafe {
                    libc::close(fifo_fd);
                    libc::close(pty_writer_fd);
                }
                return Err(format!(
                    "Failed to read runtime input fifo '{}': {err}",
                    input_path.display()
                ));
            }
        }
    }
    unsafe {
        libc::close(fifo_fd);
        libc::close(pty_writer_fd);
    }
    Ok(())
}

pub fn write_all_fd(fd: libc::c_int, payload: &[u8]) -> Result<(), String> {
    let mut offset = 0_usize;
    while offset < payload.len() {
        let write_result = unsafe {
            libc::write(
                fd,
                payload[offset..].as_ptr() as *const libc::c_void,
                payload.len() - offset,
            )
        };
        if write_result > 0 {
            offset += write_result as usize;
            continue;
        }
        if write_result == 0 {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EINTR => continue,
            Some(code) if code == libc::EPIPE || code == libc::EIO || code == libc::EBADF => {
                return Ok(());
            }
            _ => return Err(format!("Failed to write runtime PTY input: {err}")),
        }
    }
    Ok(())
}

pub fn write_pty_output_to_log(
    pty_reader_fd: libc::c_int,
    mut log_file: fs::File,
) -> Result<(), String> {
    let mut reader = unsafe { fs::File::from_raw_fd(pty_reader_fd) };
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
