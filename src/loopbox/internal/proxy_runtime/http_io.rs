use super::*;

pub(super) fn is_loopback_peer(stream: &TcpStream) -> bool {
    stream
        .peer_addr()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or(false)
}

pub(super) fn read_http_preamble(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    read_http_preamble_with_limit(stream, MAX_REQUEST_HEADER_BYTES, "client request")
}

pub(super) fn read_http_preamble_with_limit(
    stream: &mut TcpStream,
    max_bytes: usize,
    stream_label: &str,
) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 2048];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|err| format!("Failed reading {stream_label}: {err}"))?;
        if read == 0 {
            return Err(format!("{stream_label} closed before sending headers."));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if has_header_terminator(&buffer) {
            return Ok(buffer);
        }
        if buffer.len() > max_bytes {
            return Err(format!(
                "HTTP headers exceed reverse proxy limit while reading {stream_label}."
            ));
        }
    }
}

pub(super) fn has_header_terminator(bytes: &[u8]) -> bool {
    header_end_index(bytes).is_some()
}

pub(super) fn copy_stream_with_optional_preview(
    reader: &mut TcpStream,
    writer: &mut TcpStream,
    prefetched: &[u8],
    stream_until_eof: bool,
    mut preview: Option<&mut PreviewCapture>,
    stream_label: &str,
) -> Result<u64, String> {
    let mut copied = 0_u64;
    if !prefetched.is_empty() {
        if let Some(capture) = preview.as_deref_mut() {
            capture.ingest(prefetched);
        }
        writer
            .write_all(prefetched)
            .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
        copied = copied.saturating_add(prefetched.len() as u64);
    }

    if !stream_until_eof {
        return Ok(copied);
    }

    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| format!("Failed reading {stream_label}: {err}"))?;
        if read == 0 {
            break;
        }
        if let Some(capture) = preview.as_deref_mut() {
            capture.ingest(&buffer[..read]);
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
        copied = copied.saturating_add(read as u64);
    }
    Ok(copied)
}

pub(super) fn tunnel_upgraded_connection(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
) -> Result<(), String> {
    let mut client_reader = client
        .try_clone()
        .map_err(|err| format!("Failed to clone client socket for upgrade tunnel: {err}"))?;
    let mut upstream_writer = upstream
        .try_clone()
        .map_err(|err| format!("Failed to clone upstream socket for upgrade tunnel: {err}"))?;

    let upstream_to_client = thread::spawn(move || {
        relay_tunnel_stream(
            &mut client_reader,
            &mut upstream_writer,
            "upgrade tunnel client->upstream",
        )
    });

    let downstream_result =
        relay_tunnel_stream(upstream, client, "upgrade tunnel upstream->client");
    let upstream_result = upstream_to_client
        .join()
        .map_err(|_| "Upgrade tunnel thread panicked.".to_string())?;

    downstream_result?;
    upstream_result?;
    Ok(())
}

pub(super) fn relay_tunnel_stream(
    reader: &mut TcpStream,
    writer: &mut TcpStream,
    stream_label: &str,
) -> Result<(), String> {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(err) if is_connection_termination_error(&err) => 0,
            Err(err) => return Err(format!("Failed reading {stream_label}: {err}")),
        };
        if read == 0 {
            let _ = writer.shutdown(Shutdown::Write);
            return Ok(());
        }
        if let Err(err) = writer.write_all(&buffer[..read]) {
            if is_connection_termination_error(&err) {
                return Ok(());
            }
            return Err(format!("Failed writing {stream_label}: {err}"));
        }
    }
}

pub(super) fn is_connection_termination_error(err: &std::io::Error) -> bool {
    use std::io::ErrorKind;

    matches!(
        err.kind(),
        ErrorKind::BrokenPipe
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected
            | ErrorKind::UnexpectedEof
    )
}

pub(super) fn copy_fixed_body_with_optional_preview(
    reader: &mut TcpStream,
    writer: &mut TcpStream,
    prefetched: &[u8],
    expected_len: usize,
    mut preview: Option<&mut PreviewCapture>,
    stream_label: &str,
) -> Result<u64, String> {
    if expected_len == 0 {
        return Ok(0);
    }

    let mut copied = 0_u64;
    let prefetched_used = expected_len.min(prefetched.len());
    if prefetched_used > 0 {
        let bytes = &prefetched[..prefetched_used];
        if let Some(capture) = preview.as_deref_mut() {
            capture.ingest(bytes);
        }
        writer
            .write_all(bytes)
            .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
        copied = copied.saturating_add(prefetched_used as u64);
    }

    let mut remaining = expected_len.saturating_sub(prefetched_used);
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        let to_read = remaining.min(buffer.len());
        let read = reader
            .read(&mut buffer[..to_read])
            .map_err(|err| format!("Failed reading {stream_label}: {err}"))?;
        if read == 0 {
            return Err(format!(
                "Unexpected EOF while reading fixed-size {stream_label}."
            ));
        }
        if let Some(capture) = preview.as_deref_mut() {
            capture.ingest(&buffer[..read]);
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
        copied = copied.saturating_add(read as u64);
        remaining = remaining.saturating_sub(read);
    }

    Ok(copied)
}

pub(super) fn copy_chunked_body_with_optional_preview(
    reader: &mut TcpStream,
    writer: &mut TcpStream,
    prefetched: &[u8],
    mut preview: Option<&mut PreviewCapture>,
    stream_label: &str,
) -> Result<u64, String> {
    let mut copied = 0_u64;
    let mut prefetched_offset = 0_usize;
    let mut chunk_buffer = [0_u8; 16 * 1024];

    loop {
        let line = read_line_crlf_from_prefetched_or_stream(
            reader,
            prefetched,
            &mut prefetched_offset,
            stream_label,
        )?;
        writer
            .write_all(&line)
            .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
        copied = copied.saturating_add(line.len() as u64);

        let line_trimmed = line.strip_suffix(b"\r\n").unwrap_or(line.as_slice());
        let line_text = std::str::from_utf8(line_trimmed)
            .map_err(|err| format!("Invalid chunk size line in {stream_label}: {err}"))?;
        let chunk_size_hex = line_text.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(chunk_size_hex, 16).map_err(|err| {
            format!("Failed to parse chunk size '{chunk_size_hex}' in {stream_label}: {err}")
        })?;

        if chunk_size == 0 {
            loop {
                let trailer = read_line_crlf_from_prefetched_or_stream(
                    reader,
                    prefetched,
                    &mut prefetched_offset,
                    stream_label,
                )?;
                writer
                    .write_all(&trailer)
                    .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
                copied = copied.saturating_add(trailer.len() as u64);
                if trailer == b"\r\n" {
                    return Ok(copied);
                }
            }
        }

        let mut remaining = chunk_size;
        while remaining > 0 {
            let to_read = remaining.min(chunk_buffer.len());
            read_exact_from_prefetched_or_stream(
                reader,
                prefetched,
                &mut prefetched_offset,
                &mut chunk_buffer[..to_read],
                stream_label,
            )?;
            if let Some(capture) = preview.as_deref_mut() {
                capture.ingest(&chunk_buffer[..to_read]);
            }
            writer
                .write_all(&chunk_buffer[..to_read])
                .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
            copied = copied.saturating_add(to_read as u64);
            remaining -= to_read;
        }

        let mut chunk_terminator = [0_u8; 2];
        read_exact_from_prefetched_or_stream(
            reader,
            prefetched,
            &mut prefetched_offset,
            &mut chunk_terminator,
            stream_label,
        )?;
        writer
            .write_all(&chunk_terminator)
            .map_err(|err| format!("Failed writing {stream_label}: {err}"))?;
        copied = copied.saturating_add(chunk_terminator.len() as u64);
        if chunk_terminator != *b"\r\n" {
            return Err(format!(
                "Invalid chunk terminator while reading {stream_label}."
            ));
        }
    }
}

pub(super) fn read_line_crlf_from_prefetched_or_stream(
    reader: &mut TcpStream,
    prefetched: &[u8],
    prefetched_offset: &mut usize,
    stream_label: &str,
) -> Result<Vec<u8>, String> {
    let mut line = Vec::new();
    while line.len() < 32 * 1024 {
        let byte = read_byte_from_prefetched_or_stream(
            reader,
            prefetched,
            prefetched_offset,
            stream_label,
        )?;
        line.push(byte);
        if line.ends_with(b"\r\n") {
            return Ok(line);
        }
    }
    Err(format!("HTTP line too long while reading {stream_label}."))
}

pub(super) fn read_byte_from_prefetched_or_stream(
    reader: &mut TcpStream,
    prefetched: &[u8],
    prefetched_offset: &mut usize,
    stream_label: &str,
) -> Result<u8, String> {
    if *prefetched_offset < prefetched.len() {
        let byte = prefetched[*prefetched_offset];
        *prefetched_offset += 1;
        return Ok(byte);
    }
    let mut single = [0_u8; 1];
    let read = reader
        .read(&mut single)
        .map_err(|err| format!("Failed reading {stream_label}: {err}"))?;
    if read == 0 {
        return Err(format!("Unexpected EOF while reading {stream_label}."));
    }
    Ok(single[0])
}

pub(super) fn read_exact_from_prefetched_or_stream(
    reader: &mut TcpStream,
    prefetched: &[u8],
    prefetched_offset: &mut usize,
    dest: &mut [u8],
    stream_label: &str,
) -> Result<(), String> {
    let mut written = 0_usize;
    while written < dest.len() {
        if *prefetched_offset < prefetched.len() {
            let available = prefetched.len().saturating_sub(*prefetched_offset);
            let to_copy = (dest.len() - written).min(available);
            dest[written..written + to_copy]
                .copy_from_slice(&prefetched[*prefetched_offset..*prefetched_offset + to_copy]);
            *prefetched_offset += to_copy;
            written += to_copy;
            continue;
        }

        let read = reader
            .read(&mut dest[written..])
            .map_err(|err| format!("Failed reading {stream_label}: {err}"))?;
        if read == 0 {
            return Err(format!("Unexpected EOF while reading {stream_label}."));
        }
        written += read;
    }
    Ok(())
}

pub(super) fn looks_like_text_bytes(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    if bytes.contains(&0) {
        return false;
    }
    for b in bytes {
        let is_control = matches!(*b, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F);
        if is_control {
            return false;
        }
    }
    true
}
