# Loopbox Codex Agents

Loopbox includes a native **Agents** page that embeds `codex app-server` over stdio. It is intended for interactive Loopbox work: asking questions about sandboxes, reading logs, inspecting runtime state, diagnosing traffic, and approving controlled project/runtime mutations.

## Runtime Model

Loopbox starts:

```sh
codex app-server --listen stdio:// \
  -c mcp_servers.loopbox.command="<current loopbox executable>" \
  -c 'mcp_servers.loopbox.args=["__loopbox_mcp_server"]' \
  -c mcp_servers.loopbox.required=true
```

The MCP registration is passed as app-server config overrides, so Loopbox does not edit the user's global Codex config.

Loopbox reuses the user's existing Codex authentication. If `account/read` reports no usable account, the Agents page shows a sign-in state and expects the user to authenticate with the Codex CLI/app before reconnecting.

## Configuration

The following config keys live under `global.codex_agents`:

```toml
[global.codex_agents]
enabled = true
codex_binary = "/opt/homebrew/bin/codex" # optional
default_model = "gpt-5.4"
default_effort = "medium"
default_sandbox = "workspace-write"
```

`codex_binary` is optional. When omitted, Loopbox runs `codex` from `PATH`.

## MCP Tools

The hidden `loopbox __loopbox_mcp_server` subcommand exposes these MCP tools:

- `loopbox_overview`
- `loopbox_doctor`
- `loopbox_list_projects`
- `loopbox_read_project`
- `loopbox_runtime`
- `loopbox_logs`
- `loopbox_requests`
- `loopbox_resources`
- `loopbox_validate_project_config`
- `loopbox_create_project`
- `loopbox_update_project`
- `loopbox_start_project`
- `loopbox_stop_project`
- `loopbox_restart_project`
- `loopbox_start_service`
- `loopbox_stop_service`
- `loopbox_restart_service`
- `loopbox_send_service_input`

Read-only tools run directly. Mutating tools request MCP elicitation first; the Agents page renders the resulting app-server approval request and responds with Accept or Decline.

## Agent Instructions

Each new Codex thread starts with Loopbox-specific developer instructions:

- Use Loopbox MCP tools for sandbox, runtime, log, request, and resource questions.
- Prefer project hostnames returned by Loopbox over guessed localhost ports.
- Fetch logs with explicit limits.
- Ask before destructive, broad, or mutating changes.
- Use MCP elicitation for project/runtime mutations.

## Manual Check

1. Open the Agents page.
2. Start Codex and confirm auth/model status loads.
3. Ask for a sandbox summary.
4. Trigger a mutating request, decline it, and confirm no change happened.
5. Trigger another mutation, accept it, and confirm the runtime/config changed as expected.
6. Interrupt an active turn and send a follow-up message in the same thread.
