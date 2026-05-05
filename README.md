<div align="center">

# ◈ loopbox

**Local sandbox control plane for desktop development.**

One IP per project. Stable hostnames. Agent-ready debugging.

[![Rust](https://img.shields.io/badge/rust-2021-b7410e?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Dioxus](https://img.shields.io/badge/dioxus-0.7-EB4B30?style=flat-square)](https://dioxuslabs.com/)
[![Platform](https://img.shields.io/badge/platform-macOS%20%2B%20Windows%20experimental-000000?style=flat-square)](#platform-support)
[![License](https://img.shields.io/badge/license-PolyForm--Noncommercial--1.0.0-3C3C3C?style=flat-square)](LICENSE)

</div>

---

Each project gets a dedicated loopback IP and stable `.localhost` hostnames, routed through an internal reverse proxy and managed from a native desktop GUI. Host-based HTTP routes and loopback endpoint listeners for gRPC/TCP are configured from the same sandbox model. Keep URLs consistent across projects, isolate browser storage per hostname, start services, tail logs, inspect traffic, merge `.env` files, and let local AI agents control runtime + debugging workflows via the Agent API. Docker can coexist: bind containers to a sandbox IP to reuse common ports without cross-project conflicts.

```
frontend.myapp.localhost  →  127.0.0.2:5173
backend.myapp.localhost   →  127.0.0.2:8080
gateway.myapp.localhost   →  127.0.0.2:3000
127.0.0.1:50051           →  127.0.0.2:50051  (grpc_h2c)
127.0.0.1:15432           →  127.0.0.2:15432  (tcp_passthrough)
```

## Features

- **Sandbox Identity** — stable loopback IP + generated hostnames per project for predictable URLs, isolated browser storage, and managed `/etc/hosts` entries
- **Reverse Proxy Layers** — host-based HTTP routing (`service.project.localhost`) plus loopback endpoint listeners (`grpc_h2c`, `tcp_passthrough`); falls back to `:18080` with pf redirect if `:80` is unavailable
- **Docker Management** — bind containers to a sandbox IP (for example `127.0.0.30`) so multiple projects can reuse the same container ports without collisions
- **Multi-Port Services** — each service can define multiple `port + protocol + health` entries (`http1`, `grpc_h2c`, `tcp_passthrough`)
- **Process Runtime** — start/stop/restart individually or all at once; PID registry survives app restarts
- **Live Logs** — combined stdout/stderr per service, tailed in-app
- **HTTP + gRPC Traffic Inspector** — full request/response capture with filtering, HAR export, and body preview
- **Agent API Audit Log** — captures each local Agent API request/response (headers, body snapshots, status, latency) in a dedicated UI tab
- **gRPC Proto Decode** — optional project proto paths for typed payload decoding via `protoc` with `--decode_raw` fallback
- **Command Discovery** — scans `package.json` recursively; scores suggestions by service name; detects package manager from lockfiles
- **Env Management** — discovers and merges `.env*` files; injects `LOOPBOX_*` vars (`LOOPBOX_PORT_*`, `LOOPBOX_PORTS_*`, `LOOPBOX_URL_*`) into every spawned process and terminal
- **Vite Intelligence** — auto-injects `--host`, `--port`, `--strictPort`, and `__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS`
- **Terminal Integration** — process services run with a persistent macOS PTY session; the in-app terminal reconnects after Loopbox restarts, with native Terminal.app kept as a legacy fallback
- **Health Checks** — TCP port reachability + optional HTTP path and gRPC health target checks; `running` vs `unhealthy` state distinction
- **Doctor** — validates IPs, `/etc/hosts`, loopback aliases, DNS, ports, and env files; includes direct fix actions
- **Local Agent API** — localhost HTTP API for tools like Codex/Claude/Cursor (doctor, projects, create/update config, runtime, logs, requests, service input, start/stop/restart; no delete endpoint) without manual copy/paste between app and terminal

## Licensing

Loopbox is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE).

- **Free for personal use** — hobby projects, research, education, non-commercial organizations
- **Commercial use requires a paid license** — $14/seat/month or $11/seat/month yearly at [loopbox.tech/pricing](https://loopbox.tech/pricing)

All features are included in the public app. There is no runtime feature gating or license activation.

## Agent API

Loopbox exposes a local API for agent clients while the app is running.

- Discovery file: `~/.config/loopbox/agent-api.json`
- Token file: `~/.config/loopbox/agent-api-token` (when auth is enabled)
- Default URL: `http://127.0.0.1:39393`
- OpenAPI endpoint: `/v1/openapi.json` (auto-generated at runtime)
- Runtime input endpoint: `/v1/projects/{project}/services/{service}/input` for attached process services
- Edit enable/auth/port in **Settings → Agent API**

See `docs/agent-api.md` for endpoint and curl examples.

## Quick Start

**Prerequisites:** macOS primary support or experimental Windows support, [Rust toolchain](https://rustup.rs/), [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started)

```bash
cargo run
# or with hot-reload:
dx serve --platform desktop
```

If another `dx` binary is earlier in `PATH`, use `$HOME/.cargo/bin/dx` directly or set `DIOXUS_CLI_BIN` when running scripts.

The development default does not compile Ghostty's native library. To build the high-fidelity `libghostty-vt` adapter, install Zig 0.15.2 and run with `--features ghostty-vt`; the build will fetch the pinned Ghostty source unless `GHOSTTY_SOURCE_DIR` points at a local checkout.

1. Click **New Sandbox** → pick a project directory
2. Add services or hit **Auto Detect** to fill from `package.json` scripts
3. **System → Setup System** → confirm the admin prompt (one-time, reversible)
4. **Start** services, open URLs, tail logs

## Config

`~/.config/loopbox/config.toml`

```toml
[global]
domain_suffix = "localhost"
ip_base = "127.0.0."
ip_range_start = 2
ip_range_end = 254

[global.resource_metrics]
enabled = true
sample_interval_secs = 5
retention_days = 7
max_storage_mb = 250

[projects.myapp]
dir = "/Users/you/dev/myapp"
ip = "127.0.0.2"
default_open_service = "frontend"
grpc_proto_paths = ["./proto", "./apps/gateway/proto"]

[[projects.myapp.services]]
name = "backend"
command = "pnpm dev"
workdir = "/Users/you/dev/myapp/apps/backend"
env_files = [".env", ".env.local"]

[[projects.myapp.services.ports]]
port = 8080
protocol = "http1"
health_path = "/health"

[[projects.myapp.services]]
name = "postgres"
runtime = "container"
command = ""
workdir = "/Users/you/dev/myapp"

[projects.myapp.services.container]
image = "postgres:16-alpine"
env = ["POSTGRES_DB=myapp", "POSTGRES_PASSWORD=loopbox"]
volumes = ["myapp-pgdata:/var/lib/postgresql/data"]
auto_remove = true

[[projects.myapp.services.ports]]
port = 5432
protocol = "tcp_passthrough"

[[projects.myapp.services]]
name = "gateway"
command = "go run ./cmd/gateway"
workdir = "/Users/you/dev/myapp"

[[projects.myapp.services.ports]]
port = 50051
protocol = "grpc_h2c"
health_path = "my.package.Gateway"

[[projects.myapp.services.ports]]
port = 8081
protocol = "http1"

[[projects.myapp.proxy_endpoints]]
name = "gateway-grpc-alias"
listen_host = "127.0.0.1"
listen_port = 50060
protocol = "grpc_h2c"
authority = "gateway.internal.localhost"
upstream_host = "127.0.0.2"
upstream_port = 50051
service_name = "gateway"
```

Service protocols: `http1`, `grpc_h2c`, `tcp_passthrough`.

Loopbox still accepts legacy single-port service fields (`port`, `protocol`, `health_path`) and normalizes them into `services.ports` on save/load.

Resource metrics are sampled while Loopbox or the headless Agent API is running. `sample_interval_secs` is clamped to 2-60 seconds, `retention_days` to 1-90 days, and `max_storage_mb` to 25-5,000 MB.

For gRPC payload decoding, configure `grpc_proto_paths` and ensure `protoc` is available in your `PATH`.

## Platform Support

Loopbox is primarily supported on macOS. Windows support is experimental and intended for validation of the cross-platform networking/runtime path.

- **macOS:** loopback aliases on `lo0`, managed `/etc/hosts` entries, `pf` redirect rules for domain-only HTTP, Sparkle updates, Terminal.app launch, and persistent PTY-backed integrated terminal sessions for process services.
- **Windows:** loopback aliases through `netsh interface ipv4`, managed `C:\Windows\System32\drivers\etc\hosts` entries, `netsh interface portproxy` rules for domain-only HTTP, native folder dialogs, and standard process start/stop/log follow.
- **Current Windows gaps:** PTY attach/input flows and auto-update are not yet available. Use standard process mode and manual downloads while Windows support remains experimental.

See `docs/windows.md` for the current Windows setup notes.

## Tech Stack

| | |
|---|---|
| **Language** | Rust 2021 |
| **UI** | Dioxus 0.7 (native desktop) |
| **Async** | Tokio |
| **Config** | TOML (`~/.config/loopbox/`) |
| **Styling** | Tailwind CSS, Sora + JetBrains Mono |
| **Platform** | macOS primary; Windows experimental |

## Build

```bash
dx build --platform desktop --release
dx bundle --platform macos --package-types dmg --release
```

The release scripts prefer `DIOXUS_CLI_BIN`, then `$HOME/.cargo/bin/dx`, then `dx` on `PATH`.
Release/update tooling lives in this repo under `scripts/`; see `docs/updater/README.md`.

`libghostty-vt` is optional at build time because its native `libghostty-vt-sys` build requires Zig and the pinned Ghostty source. Use `cargo check --features ghostty-vt` or `dx serve --platform desktop --features ghostty-vt` only after installing Zig 0.15.2. Release packaging that enables this feature must bundle and codesign the resulting `libghostty-vt.dylib` if the build links it dynamically.

## Author

**Niklas Schmidt** — [niklasschmidt.dev](https://niklasschmidt.dev/) | niklas@niklasschmidt.dev

## License

Loopbox is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). Free for personal use. Commercial use requires a [paid commercial license](https://loopbox.tech/pricing).
