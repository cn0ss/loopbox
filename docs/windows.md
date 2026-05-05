# Windows Support Notes

Windows support is experimental. The macOS build remains the primary supported target, but the Windows platform layer can validate the core sandbox model without requiring macOS-specific tools.

## Supported

- Loopback aliases through `netsh interface ipv4 add address "Loopback Pseudo-Interface 1"`.
- Managed hosts entries in `C:\Windows\System32\drivers\etc\hosts`.
- Domain-only HTTP forwarding through `netsh interface portproxy`.
- DNS refresh through `ipconfig /flushdns`.
- Native folder selection through PowerShell and `System.Windows.Forms`.
- Standard process start, stop, status, and log follow.

## Current Gaps

- Persistent integrated terminal sessions are macOS-only in v1.
- PTY-backed attach and raw input are not supported on Windows.
- FIFO-based terminal input is not supported on Windows.
- Auto-update is not available on Windows; download new builds manually.
- Windows packaging is separate from the macOS Sparkle release flow.

## Setup Expectations

Run System Setup from an elevated prompt/UAC flow. The generated script updates loopback aliases, rewrites only the managed Loopbox hosts block, configures `portproxy` rules for sandbox IPs that have HTTP services, and flushes DNS.

Use standard process mode for services that need to run on Windows. Interactive tools that depend on a PTY should be run directly in a terminal until a ConPTY-backed runtime is implemented.
