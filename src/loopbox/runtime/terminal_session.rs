use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const TERMINAL_BACKEND_VERSION: &str = "libghostty-vt-v1";
const MAX_PROTOCOL_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalFrame {
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminalMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalKeyAction {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseKind {
    Down,
    Up,
    Move,
    Wheel { delta_x: f64, delta_y: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerminalClientMessage {
    Snapshot,
    Key {
        code: String,
        text: Option<String>,
        mods: TerminalMods,
        action: TerminalKeyAction,
    },
    Paste {
        text: String,
    },
    Mouse {
        kind: TerminalMouseKind,
        x_px: f64,
        y_px: f64,
        button: Option<u8>,
        mods: TerminalMods,
    },
    Focus {
        focused: bool,
    },
    Resize {
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    Scroll {
        rows: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerminalServerMessage {
    Snapshot(TerminalFrame),
    Frame(TerminalFrame),
    Title { title: String },
    Error { message: String },
}

#[derive(Debug, Clone)]
pub(super) struct TerminalSessionArgs {
    pub workdir: String,
    pub command: String,
    pub log_file: PathBuf,
    pub input_path: PathBuf,
    pub control_path: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

#[derive(Debug, Clone)]
pub(super) struct TerminalSessionError {
    pub message: String,
    pub service_started: bool,
}

impl TerminalSessionError {
    fn before_start(message: String) -> Self {
        Self {
            message,
            service_started: false,
        }
    }

    fn after_start(message: String) -> Self {
        Self {
            message,
            service_started: true,
        }
    }
}

pub fn encode_terminal_protocol_message<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(message)
        .map_err(|err| format!("Failed to encode terminal protocol JSON: {err}"))?;
    if body.len() > MAX_PROTOCOL_MESSAGE_BYTES {
        return Err(format!(
            "Terminal protocol message is too large ({} bytes).",
            body.len()
        ));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| "Terminal protocol message length overflowed u32.".to_string())?;
    let mut encoded = Vec::with_capacity(4 + body.len());
    encoded.extend_from_slice(&len.to_be_bytes());
    encoded.extend_from_slice(&body);
    Ok(encoded)
}

pub fn decode_terminal_protocol_message<T: DeserializeOwned>(payload: &[u8]) -> Result<T, String> {
    if payload.len() < 4 {
        return Err("Terminal protocol payload is missing its length prefix.".to_string());
    }
    let declared_len =
        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if declared_len > MAX_PROTOCOL_MESSAGE_BYTES {
        return Err(format!(
            "Terminal protocol message is too large ({declared_len} bytes)."
        ));
    }
    let body = payload
        .get(4..4 + declared_len)
        .ok_or_else(|| "Terminal protocol payload ended before the JSON body.".to_string())?;
    if payload.len() != 4 + declared_len {
        return Err("Terminal protocol payload has trailing bytes.".to_string());
    }
    serde_json::from_slice(body)
        .map_err(|err| format!("Failed to decode terminal protocol JSON: {err}"))
}

pub fn terminal_session_snapshot(
    project_name: &str,
    service_name: &str,
) -> Result<TerminalFrame, String> {
    let key = super::runtime_key(project_name, service_name);
    let path = super::resolve_runtime_terminal_path_for_key(&key)?.ok_or_else(|| {
        format!("Service '{service_name}' in project '{project_name}' has no terminal session.")
    })?;
    match send_terminal_client_message_to_path(&path, &TerminalClientMessage::Snapshot)? {
        TerminalServerMessage::Snapshot(frame) | TerminalServerMessage::Frame(frame) => Ok(frame),
        TerminalServerMessage::Error { message } => Err(message),
        TerminalServerMessage::Title { .. } => Err(format!(
            "Terminal session for '{service_name}' returned a title update instead of a snapshot."
        )),
    }
}

pub fn send_terminal_client_message(
    project_name: &str,
    service_name: &str,
    message: TerminalClientMessage,
) -> Result<TerminalServerMessage, String> {
    let key = super::runtime_key(project_name, service_name);
    let path = super::resolve_runtime_terminal_path_for_key(&key)?.ok_or_else(|| {
        format!("Service '{service_name}' in project '{project_name}' has no terminal session.")
    })?;
    send_terminal_client_message_to_path(&path, &message)
}

#[cfg(unix)]
pub(super) fn send_terminal_client_message_to_path(
    path: &Path,
    message: &TerminalClientMessage,
) -> Result<TerminalServerMessage, String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(path).map_err(|err| {
        format!(
            "Failed to connect to terminal socket '{}': {err}",
            path.display()
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_millis(700)))
        .map_err(|err| format!("Failed to configure terminal socket timeout: {err}"))?;
    let payload = encode_terminal_protocol_message(message)?;
    stream
        .write_all(&payload)
        .map_err(|err| format!("Failed to write terminal protocol message: {err}"))?;
    stream
        .flush()
        .map_err(|err| format!("Failed to flush terminal protocol message: {err}"))?;

    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|err| format!("Failed to read terminal protocol response header: {err}"))?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_PROTOCOL_MESSAGE_BYTES {
        return Err(format!(
            "Terminal protocol response is too large ({len} bytes)."
        ));
    }
    let mut body = vec![0_u8; len];
    stream
        .read_exact(&mut body)
        .map_err(|err| format!("Failed to read terminal protocol response body: {err}"))?;
    let mut framed = Vec::with_capacity(4 + len);
    framed.extend_from_slice(&header);
    framed.extend_from_slice(&body);
    decode_terminal_protocol_message(&framed)
}

#[cfg(not(unix))]
pub(super) fn send_terminal_client_message_to_path(
    _path: &Path,
    _message: &TerminalClientMessage,
) -> Result<TerminalServerMessage, String> {
    Err("Integrated terminal sockets are not supported on this platform.".to_string())
}

#[cfg(unix)]
pub(super) fn run_terminal_session(args: TerminalSessionArgs) -> Result<i32, TerminalSessionError> {
    session_unix::run_terminal_session(args)
}

#[cfg(not(unix))]
pub(super) fn run_terminal_session(
    _args: TerminalSessionArgs,
) -> Result<i32, TerminalSessionError> {
    Err(TerminalSessionError::before_start(
        "Integrated terminal sessions are not supported on this platform.".to_string(),
    ))
}

#[cfg(unix)]
mod session_unix {
    use super::*;
    #[cfg(not(feature = "ghostty-vt"))]
    use std::collections::VecDeque;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::thread;
    use std::time::{Duration, Instant};

    struct TerminalClient {
        stream: UnixStream,
        buffer: Vec<u8>,
    }

    pub(super) fn run_terminal_session(
        args: TerminalSessionArgs,
    ) -> Result<i32, TerminalSessionError> {
        if let Some(parent) = args.log_file.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                TerminalSessionError::before_start(format!(
                    "Failed to create {}: {err}",
                    parent.display()
                ))
            })?;
        }
        let log_file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&args.log_file)
            .map_err(|err| {
                TerminalSessionError::before_start(format!(
                    "Failed to open runtime terminal log file '{}': {err}",
                    args.log_file.display()
                ))
            })?;

        if let Some(parent) = args.control_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                TerminalSessionError::before_start(format!(
                    "Failed to create {}: {err}",
                    parent.display()
                ))
            })?;
        }
        let _ = fs::remove_file(&args.control_path);
        let listener = UnixListener::bind(&args.control_path).map_err(|err| {
            TerminalSessionError::before_start(format!(
                "Failed to bind runtime terminal socket '{}': {err}",
                args.control_path.display()
            ))
        })?;
        listener.set_nonblocking(true).map_err(|err| {
            TerminalSessionError::before_start(format!(
                "Failed to configure terminal socket: {err}"
            ))
        })?;

        let input_fd = crate::platform::runtime::open_fifo_read_nonblocking(&args.input_path)
            .map_err(TerminalSessionError::before_start)?;

        let terminal = TerminalCore::new(
            args.cols,
            args.rows,
            args.cell_width_px,
            args.cell_height_px,
        )
        .map_err(TerminalSessionError::before_start)?;

        let (pty_fd, child_pid) = crate::platform::runtime::spawn_pty_child(
            &args.command,
            &args.workdir,
            args.cols,
            args.rows,
        )
        .map_err(TerminalSessionError::before_start)?;
        if let Err(err) = set_fd_nonblocking(pty_fd) {
            terminate_child(child_pid);
            crate::platform::runtime::close_fd(input_fd);
            crate::platform::runtime::close_fd(pty_fd);
            let _ = fs::remove_file(&args.control_path);
            return Err(TerminalSessionError::after_start(err));
        }

        match run_started_terminal_session(
            &args, log_file, listener, input_fd, pty_fd, child_pid, terminal,
        ) {
            Ok(exit_code) => Ok(exit_code),
            Err(err) => {
                terminate_child(child_pid);
                crate::platform::runtime::close_fd(input_fd);
                crate::platform::runtime::close_fd(pty_fd);
                let _ = fs::remove_file(&args.control_path);
                Err(TerminalSessionError::after_start(err))
            }
        }
    }

    fn run_started_terminal_session(
        args: &TerminalSessionArgs,
        mut log_file: fs::File,
        listener: UnixListener,
        input_fd: libc::c_int,
        pty_fd: libc::c_int,
        child_pid: libc::pid_t,
        mut terminal: TerminalCore,
    ) -> Result<i32, String> {
        let mut clients = Vec::<TerminalClient>::new();
        let mut last_frame_sent = Instant::now()
            .checked_sub(Duration::from_millis(100))
            .unwrap_or_else(Instant::now);
        let mut dirty = true;
        let mut exit_code = None::<i32>;
        let mut pty_buffer = [0_u8; 8192];
        let mut fifo_buffer = [0_u8; 2048];

        loop {
            accept_clients(&listener, &mut clients)?;

            let mut remove_clients = Vec::new();
            for (index, client) in clients.iter_mut().enumerate() {
                match read_client_messages(client) {
                    Ok(messages) => {
                        for message in messages {
                            match handle_client_message(
                                message,
                                &mut terminal,
                                pty_fd,
                                &mut client.stream,
                            ) {
                                Ok(message_dirty) => dirty |= message_dirty,
                                Err(err) => {
                                    let _ = send_server_message(
                                        &mut client.stream,
                                        &TerminalServerMessage::Error { message: err },
                                    );
                                }
                            }
                        }
                    }
                    Err(ClientReadError::Disconnected) => remove_clients.push(index),
                    Err(ClientReadError::Protocol(err)) => {
                        let _ = send_server_message(
                            &mut client.stream,
                            &TerminalServerMessage::Error { message: err },
                        );
                        remove_clients.push(index);
                    }
                }
            }
            for index in remove_clients.into_iter().rev() {
                clients.remove(index);
            }

            loop {
                match read_fd(pty_fd, &mut pty_buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = &pty_buffer[..n];
                        log_file.write_all(bytes).map_err(|err| {
                            format!("Failed to append runtime terminal log output: {err}")
                        })?;
                        let responses = terminal.feed(bytes)?;
                        for response in responses {
                            crate::platform::runtime::write_all_fd(pty_fd, &response)?;
                        }
                        dirty = true;
                    }
                    Err(ReadFdError::WouldBlock) => break,
                    Err(ReadFdError::Closed) => {
                        exit_code = Some(exit_code.unwrap_or(1));
                        break;
                    }
                    Err(ReadFdError::Other(err)) => return Err(err),
                }
            }

            loop {
                match read_fd(input_fd, &mut fifo_buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        crate::platform::runtime::write_all_fd(pty_fd, &fifo_buffer[..n])?;
                    }
                    Err(ReadFdError::WouldBlock) => break,
                    Err(ReadFdError::Closed) => break,
                    Err(ReadFdError::Other(err)) => {
                        eprintln!("Loopbox runtime terminal fifo warning: {err}");
                        break;
                    }
                }
            }

            if exit_code.is_none() {
                exit_code = try_wait_child(child_pid)?;
            }

            if dirty && last_frame_sent.elapsed() >= Duration::from_millis(33) {
                match terminal.snapshot() {
                    Ok(frame) => {
                        broadcast_server_message(
                            &mut clients,
                            &TerminalServerMessage::Frame(frame),
                        );
                    }
                    Err(err) => {
                        eprintln!("Loopbox runtime terminal render warning: {err}");
                    }
                }
                last_frame_sent = Instant::now();
                dirty = false;
            }

            if let Some(code) = exit_code {
                let frame = terminal.snapshot().unwrap_or_else(|_| TerminalFrame {
                    cols: args.cols,
                    rows: args.rows,
                    title: String::new(),
                    cursor_x: 0,
                    cursor_y: 0,
                    lines: Vec::new(),
                });
                broadcast_server_message(&mut clients, &TerminalServerMessage::Frame(frame));
                let _ = fs::remove_file(&args.control_path);
                crate::platform::runtime::close_fd(input_fd);
                crate::platform::runtime::close_fd(pty_fd);
                return Ok(code);
            }

            let _ = log_file.flush();
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate_child(pid: libc::pid_t) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        for _ in 0..8 {
            if try_wait_child(pid).ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(40));
        }
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        let _ = try_wait_child(pid);
    }

    fn accept_clients(
        listener: &UnixListener,
        clients: &mut Vec<TerminalClient>,
    ) -> Result<(), String> {
        loop {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    stream
                        .set_nonblocking(true)
                        .map_err(|err| format!("Failed to configure terminal client: {err}"))?;
                    clients.push(TerminalClient {
                        stream,
                        buffer: Vec::new(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(format!("Failed to accept terminal client: {err}")),
            }
        }
    }

    enum ClientReadError {
        Disconnected,
        Protocol(String),
    }

    fn read_client_messages(
        client: &mut TerminalClient,
    ) -> Result<Vec<TerminalClientMessage>, ClientReadError> {
        let mut temp = [0_u8; 4096];
        loop {
            match client.stream.read(&mut temp) {
                Ok(0) => return Err(ClientReadError::Disconnected),
                Ok(n) => client.buffer.extend_from_slice(&temp[..n]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    return Err(ClientReadError::Protocol(format!(
                        "Failed to read terminal client message: {err}"
                    )))
                }
            }
        }

        let mut messages = Vec::new();
        loop {
            if client.buffer.len() < 4 {
                break;
            }
            let len = u32::from_be_bytes([
                client.buffer[0],
                client.buffer[1],
                client.buffer[2],
                client.buffer[3],
            ]) as usize;
            if len > MAX_PROTOCOL_MESSAGE_BYTES {
                return Err(ClientReadError::Protocol(format!(
                    "Terminal protocol message is too large ({len} bytes)."
                )));
            }
            if client.buffer.len() < 4 + len {
                break;
            }
            let frame = client.buffer.drain(..4 + len).collect::<Vec<_>>();
            let message =
                decode_terminal_protocol_message(&frame).map_err(ClientReadError::Protocol)?;
            messages.push(message);
        }
        Ok(messages)
    }

    fn handle_client_message(
        message: TerminalClientMessage,
        terminal: &mut TerminalCore,
        pty_fd: libc::c_int,
        stream: &mut UnixStream,
    ) -> Result<bool, String> {
        match message {
            TerminalClientMessage::Snapshot => {
                send_server_message(
                    stream,
                    &TerminalServerMessage::Snapshot(terminal.snapshot()?),
                )?;
                Ok(false)
            }
            TerminalClientMessage::Key {
                code,
                text,
                mods,
                action,
            } => {
                if action == TerminalKeyAction::Release {
                    send_server_message(
                        stream,
                        &TerminalServerMessage::Frame(terminal.snapshot()?),
                    )?;
                    return Ok(false);
                }
                if let Some(bytes) = encode_key_input(&code, text.as_deref(), mods) {
                    crate::platform::runtime::write_all_fd(pty_fd, &bytes)?;
                }
                send_server_message(stream, &TerminalServerMessage::Frame(terminal.snapshot()?))?;
                Ok(false)
            }
            TerminalClientMessage::Paste { text } => {
                crate::platform::runtime::write_all_fd(pty_fd, text.as_bytes())?;
                send_server_message(stream, &TerminalServerMessage::Frame(terminal.snapshot()?))?;
                Ok(false)
            }
            TerminalClientMessage::Resize {
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            } => {
                let cols = cols.clamp(20, 500);
                let rows = rows.clamp(5, 200);
                crate::platform::runtime::resize_pty(
                    pty_fd,
                    cols,
                    rows,
                    cell_width_px,
                    cell_height_px,
                )?;
                terminal.resize(cols, rows, cell_width_px, cell_height_px)?;
                send_server_message(stream, &TerminalServerMessage::Frame(terminal.snapshot()?))?;
                Ok(true)
            }
            TerminalClientMessage::Focus { focused } => {
                let payload = if focused { b"\x1b[I" } else { b"\x1b[O" };
                let _ = crate::platform::runtime::write_all_fd(pty_fd, payload);
                send_server_message(stream, &TerminalServerMessage::Frame(terminal.snapshot()?))?;
                Ok(false)
            }
            TerminalClientMessage::Scroll { rows } => {
                terminal.scroll(rows);
                send_server_message(stream, &TerminalServerMessage::Frame(terminal.snapshot()?))?;
                Ok(true)
            }
            TerminalClientMessage::Mouse { .. } => {
                send_server_message(stream, &TerminalServerMessage::Frame(terminal.snapshot()?))?;
                Ok(false)
            }
        }
    }

    fn send_server_message(
        stream: &mut UnixStream,
        message: &TerminalServerMessage,
    ) -> Result<(), String> {
        let payload = encode_terminal_protocol_message(message)?;
        stream
            .write_all(&payload)
            .map_err(|err| format!("Failed to write terminal server message: {err}"))
    }

    fn broadcast_server_message(
        clients: &mut Vec<TerminalClient>,
        message: &TerminalServerMessage,
    ) {
        let mut remove = Vec::new();
        for (index, client) in clients.iter_mut().enumerate() {
            if send_server_message(&mut client.stream, message).is_err() {
                remove.push(index);
            }
        }
        for index in remove.into_iter().rev() {
            clients.remove(index);
        }
    }

    fn encode_key_input(code: &str, text: Option<&str>, mods: TerminalMods) -> Option<Vec<u8>> {
        let mut bytes = match code {
            "Enter" | "NumpadEnter" => b"\r".to_vec(),
            "Backspace" => vec![0x7f],
            "Tab" => b"\t".to_vec(),
            "Escape" => vec![0x1b],
            "ArrowUp" => b"\x1b[A".to_vec(),
            "ArrowDown" => b"\x1b[B".to_vec(),
            "ArrowRight" => b"\x1b[C".to_vec(),
            "ArrowLeft" => b"\x1b[D".to_vec(),
            "Home" => b"\x1b[H".to_vec(),
            "End" => b"\x1b[F".to_vec(),
            "PageUp" => b"\x1b[5~".to_vec(),
            "PageDown" => b"\x1b[6~".to_vec(),
            "Delete" => b"\x1b[3~".to_vec(),
            _ => {
                if let Some(text) = text {
                    if mods.ctrl {
                        ctrl_modified_bytes(text).unwrap_or_else(|| text.as_bytes().to_vec())
                    } else {
                        text.as_bytes().to_vec()
                    }
                } else {
                    return None;
                }
            }
        };
        if (mods.alt || mods.meta) && bytes.first().copied() != Some(0x1b) {
            let mut prefixed = Vec::with_capacity(bytes.len() + 1);
            prefixed.push(0x1b);
            prefixed.extend_from_slice(&bytes);
            bytes = prefixed;
        }
        Some(bytes)
    }

    fn ctrl_modified_bytes(text: &str) -> Option<Vec<u8>> {
        let mut chars = text.chars();
        let ch = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() {
            Some(vec![(lower as u8) - b'a' + 1])
        } else {
            match lower {
                '[' => Some(vec![0x1b]),
                '\\' => Some(vec![0x1c]),
                ']' => Some(vec![0x1d]),
                '^' => Some(vec![0x1e]),
                '_' => Some(vec![0x1f]),
                _ => None,
            }
        }
    }

    enum ReadFdError {
        WouldBlock,
        Closed,
        Other(String),
    }

    fn read_fd(fd: libc::c_int, buffer: &mut [u8]) -> Result<usize, ReadFdError> {
        let read_count =
            unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len()) };
        if read_count > 0 {
            return Ok(read_count as usize);
        }
        if read_count == 0 {
            return Ok(0);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EINTR || code == libc::EAGAIN => {
                Err(ReadFdError::WouldBlock)
            }
            Some(code) if code == libc::EIO || code == libc::EBADF => Err(ReadFdError::Closed),
            _ => Err(ReadFdError::Other(format!(
                "Failed to read runtime terminal PTY: {err}"
            ))),
        }
    }

    fn set_fd_nonblocking(fd: libc::c_int) -> Result<(), String> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(format!(
                "Failed to read terminal fd flags: {}",
                std::io::Error::last_os_error()
            ));
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(format!(
                "Failed to set terminal fd nonblocking: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn try_wait_child(pid: libc::pid_t) -> Result<Option<i32>, String> {
        let mut status = 0_i32;
        let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if waited == 0 {
            return Ok(None);
        }
        if waited == pid {
            if libc::WIFEXITED(status) {
                return Ok(Some(libc::WEXITSTATUS(status) as i32));
            }
            if libc::WIFSIGNALED(status) {
                return Ok(Some(128 + libc::WTERMSIG(status) as i32));
            }
            return Ok(Some(1));
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ECHILD) {
            return Ok(Some(0));
        }
        Err(format!(
            "Failed while waiting for runtime terminal child {pid}: {err}"
        ))
    }

    struct TerminalCore {
        inner: TerminalCoreInner,
    }

    impl TerminalCore {
        fn new(
            cols: u16,
            rows: u16,
            cell_width_px: u32,
            cell_height_px: u32,
        ) -> Result<Self, String> {
            Ok(Self {
                inner: TerminalCoreInner::new(cols, rows, cell_width_px, cell_height_px)?,
            })
        }

        fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
            self.inner.feed(bytes)
        }

        fn resize(
            &mut self,
            cols: u16,
            rows: u16,
            cell_width_px: u32,
            cell_height_px: u32,
        ) -> Result<(), String> {
            self.inner.resize(cols, rows, cell_width_px, cell_height_px)
        }

        fn scroll(&mut self, rows: i32) {
            self.inner.scroll(rows);
        }

        fn snapshot(&mut self) -> Result<TerminalFrame, String> {
            self.inner.snapshot()
        }
    }

    #[cfg(all(test, feature = "ghostty-vt"))]
    mod tests {
        use super::*;

        #[test]
        fn ghostty_snapshot_handles_empty_terminal() {
            let mut terminal = TerminalCore::new(80, 24, 9, 18).expect("terminal");
            let frame = terminal.snapshot().expect("empty snapshot");

            assert_eq!(frame.cols, 80);
            assert_eq!(frame.rows, 24);
            assert_eq!(frame.lines.len(), 24);
        }
    }

    #[cfg(feature = "ghostty-vt")]
    struct TerminalCoreInner {
        terminal: libghostty_vt::Terminal<'static, 'static>,
        render_state: libghostty_vt::RenderState<'static>,
        pty_writes: std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>,
        cols: u16,
        rows: u16,
        title: String,
    }

    #[cfg(feature = "ghostty-vt")]
    impl TerminalCoreInner {
        fn new(
            cols: u16,
            rows: u16,
            _cell_width_px: u32,
            _cell_height_px: u32,
        ) -> Result<Self, String> {
            let pty_writes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut terminal = libghostty_vt::Terminal::new(libghostty_vt::TerminalOptions {
                cols,
                rows,
                max_scrollback: 10_000,
            })
            .map_err(|err| format!("Failed to create libghostty terminal: {err:?}"))?;
            terminal
                .on_pty_write({
                    let pty_writes = std::rc::Rc::clone(&pty_writes);
                    move |_terminal, data| {
                        pty_writes.borrow_mut().push(data.to_vec());
                    }
                })
                .map_err(|err| format!("Failed to configure libghostty PTY callback: {err:?}"))?;
            let render_state = libghostty_vt::RenderState::new()
                .map_err(|err| format!("Failed to create libghostty render state: {err:?}"))?;
            Ok(Self {
                terminal,
                render_state,
                pty_writes,
                cols,
                rows,
                title: String::new(),
            })
        }

        fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
            self.terminal.vt_write(bytes);
            self.title = self.terminal.title().unwrap_or_default().to_string();
            Ok(std::mem::take(&mut *self.pty_writes.borrow_mut()))
        }

        fn resize(
            &mut self,
            cols: u16,
            rows: u16,
            cell_width_px: u32,
            cell_height_px: u32,
        ) -> Result<(), String> {
            self.terminal
                .resize(cols, rows, cell_width_px, cell_height_px)
                .map_err(|err| format!("Failed to resize libghostty terminal: {err:?}"))?;
            self.cols = cols;
            self.rows = rows;
            Ok(())
        }

        fn scroll(&mut self, _rows: i32) {}

        fn snapshot(&mut self) -> Result<TerminalFrame, String> {
            let snapshot = self
                .render_state
                .update(&self.terminal)
                .map_err(|err| format!("Failed to update libghostty render state: {err:?}"))?;
            let cols = snapshot.cols().unwrap_or(self.cols);
            let rows = snapshot.rows().unwrap_or(self.rows);
            let cursor = snapshot.cursor_viewport().ok().flatten();

            let mut row_handle = libghostty_vt::render::RowIterator::new()
                .map_err(|err| format!("Failed to create libghostty row iterator: {err:?}"))?;
            let mut cell_handle = libghostty_vt::render::CellIterator::new()
                .map_err(|err| format!("Failed to create libghostty cell iterator: {err:?}"))?;
            let mut row_iter = row_handle
                .update(&snapshot)
                .map_err(|err| format!("Failed to read libghostty rows: {err:?}"))?;
            let mut lines = Vec::new();

            while let Some(row) = row_iter.next() {
                let mut cell_iter = cell_handle
                    .update(row)
                    .map_err(|err| format!("Failed to read libghostty row cells: {err:?}"))?;
                let mut line = String::new();
                while let Some(cell) = cell_iter.next() {
                    let graphemes = cell
                        .graphemes()
                        .map_err(|err| format!("Failed to read libghostty cell text: {err:?}"))?;
                    for ch in graphemes {
                        if ch != '\0' {
                            line.push(ch);
                        }
                    }
                }
                lines.push(line);
            }
            fit_lines(&mut lines, rows);
            Ok(TerminalFrame {
                cols,
                rows,
                title: self.title.clone(),
                cursor_x: cursor.map(|cursor| cursor.x).unwrap_or(0),
                cursor_y: cursor.map(|cursor| cursor.y).unwrap_or(0),
                lines,
            })
        }
    }

    #[cfg(not(feature = "ghostty-vt"))]
    struct TerminalCoreInner {
        cols: u16,
        rows: u16,
        title: String,
        cursor_x: u16,
        cursor_y: u16,
        lines: VecDeque<String>,
        scrollback_offset: i32,
    }

    #[cfg(not(feature = "ghostty-vt"))]
    impl TerminalCoreInner {
        fn new(
            cols: u16,
            rows: u16,
            _cell_width_px: u32,
            _cell_height_px: u32,
        ) -> Result<Self, String> {
            let mut lines = VecDeque::new();
            lines.push_back(String::new());
            Ok(Self {
                cols,
                rows,
                title: String::new(),
                cursor_x: 0,
                cursor_y: 0,
                lines,
                scrollback_offset: 0,
            })
        }

        fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
            self.process_text(&String::from_utf8_lossy(bytes));
            Ok(Vec::new())
        }

        fn resize(
            &mut self,
            cols: u16,
            rows: u16,
            _cell_width_px: u32,
            _cell_height_px: u32,
        ) -> Result<(), String> {
            self.cols = cols;
            self.rows = rows;
            self.trim_scrollback();
            Ok(())
        }

        fn scroll(&mut self, rows: i32) {
            self.scrollback_offset = self.scrollback_offset.saturating_add(rows);
            self.scrollback_offset = self.scrollback_offset.clamp(
                -(self.lines.len() as i32),
                self.lines.len().saturating_sub(self.rows as usize) as i32,
            );
        }

        fn snapshot(&mut self) -> Result<TerminalFrame, String> {
            let total = self.lines.len();
            let viewport_rows = self.rows as usize;
            let base_start = total.saturating_sub(viewport_rows);
            let start = if self.scrollback_offset < 0 {
                base_start.saturating_sub(self.scrollback_offset.unsigned_abs() as usize)
            } else {
                base_start.saturating_add(self.scrollback_offset as usize)
            }
            .min(total);
            let mut lines = self
                .lines
                .iter()
                .skip(start)
                .take(viewport_rows)
                .cloned()
                .collect::<Vec<_>>();
            fit_lines(&mut lines, self.rows);
            Ok(TerminalFrame {
                cols: self.cols,
                rows: self.rows,
                title: self.title.clone(),
                cursor_x: self.cursor_x.min(self.cols.saturating_sub(1)),
                cursor_y: self.cursor_y.min(self.rows.saturating_sub(1)),
                lines,
            })
        }

        fn process_text(&mut self, text: &str) {
            let mut chars = text.chars().peekable();
            while let Some(ch) = chars.next() {
                match ch {
                    '\u{1b}' => self.consume_escape(&mut chars),
                    '\r' => self.cursor_x = 0,
                    '\n' => self.newline(),
                    '\u{8}' => self.cursor_x = self.cursor_x.saturating_sub(1),
                    '\t' => {
                        let next_tab = ((self.cursor_x / 8) + 1) * 8;
                        while self.cursor_x < next_tab {
                            self.put_char(' ');
                        }
                    }
                    '\u{7}' => {}
                    ch if !ch.is_control() => self.put_char(ch),
                    _ => {}
                }
            }
            self.trim_scrollback();
        }

        fn consume_escape<I>(&mut self, chars: &mut std::iter::Peekable<I>)
        where
            I: Iterator<Item = char>,
        {
            match chars.next() {
                Some('[') => {
                    let mut command = String::new();
                    for ch in chars.by_ref() {
                        command.push(ch);
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                    self.apply_csi(&command);
                }
                Some(']') => {
                    let mut osc = String::new();
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' {
                            break;
                        }
                        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
                            let _ = chars.next();
                            break;
                        }
                        osc.push(ch);
                    }
                    if let Some(title) = osc
                        .strip_prefix("0;")
                        .or_else(|| osc.strip_prefix("2;"))
                        .or_else(|| osc.strip_prefix("1;"))
                    {
                        self.title = title.to_string();
                    }
                }
                _ => {}
            }
        }

        fn apply_csi(&mut self, command: &str) {
            if command.ends_with('J') {
                self.lines.clear();
                self.lines.push_back(String::new());
                self.cursor_x = 0;
                self.cursor_y = 0;
            } else if command.ends_with('K') {
                if let Some(line) = self.lines.back_mut() {
                    line.truncate(self.cursor_x as usize);
                }
            } else if command.ends_with('H') || command.ends_with('f') {
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
        }

        fn put_char(&mut self, ch: char) {
            if self.lines.is_empty() {
                self.lines.push_back(String::new());
            }
            if self.cursor_x >= self.cols {
                self.newline();
            }
            let line = self.lines.back_mut().expect("line exists");
            line.push(ch);
            self.cursor_x = self.cursor_x.saturating_add(1);
        }

        fn newline(&mut self) {
            self.lines.push_back(String::new());
            self.cursor_x = 0;
            self.cursor_y = self
                .cursor_y
                .saturating_add(1)
                .min(self.rows.saturating_sub(1));
        }

        fn trim_scrollback(&mut self) {
            let max_lines = 10_000_usize.max(self.rows as usize);
            while self.lines.len() > max_lines {
                self.lines.pop_front();
            }
        }
    }

    fn fit_lines(lines: &mut Vec<String>, rows: u16) {
        let rows = rows as usize;
        if lines.len() > rows {
            let skip = lines.len() - rows;
            lines.drain(..skip);
        }
        while lines.len() < rows {
            lines.push(String::new());
        }
    }
}
