# Loopbox Agent API (Local)

Loopbox now exposes a local HTTP API for agent tools.

For the native Codex-powered chat inside Loopbox, see
[`docs/codex-agents.md`](codex-agents.md). The HTTP Agent API remains available
for external tools and scripts.

## Base URL and token

- Discovery file: `~/.config/loopbox/agent-api.json`
- Token file: `~/.config/loopbox/agent-api-token` (only when auth is enabled)
- Default bind: `127.0.0.1:39393`
- Port/auth/enabled are editable in Loopbox Settings

Example auth header:

```sh
TOKEN="$(cat ~/.config/loopbox/agent-api-token)"
AUTH="Authorization: Bearer ${TOKEN}"
```

## Endpoints

- `GET /v1/health` (no auth)
- `GET /v1/meta`
- `GET /v1/doctor`
- `GET /v1/projects`
- `POST /v1/projects?apply_system_setup={true|false}`
- `GET /v1/projects/{project}`
- `PUT /v1/projects/{project}?apply_system_setup={true|false}`
- `GET /v1/projects/{project}/runtime`
- `GET /v1/projects/{project}/incidents?service={service?}&window={15m|1h|24h|7d}&limit={n}`
- `GET /v1/projects/{project}/resources?service={service?}&window={15m|1h|24h|7d}&limit={n}`
- `GET /v1/projects/{project}/logs?service={service}&limit={n}`
- `GET /v1/projects/{project}/requests?service={service?}&limit={n}`
- `POST /v1/projects/{project}/start`
- `POST /v1/projects/{project}/stop`
- `POST /v1/projects/{project}/restart`
- `POST /v1/projects/{project}/services/{service}/start`
- `POST /v1/projects/{project}/services/{service}/stop`
- `POST /v1/projects/{project}/services/{service}/restart`
- `POST /v1/projects/{project}/services/{service}/input`

OpenAPI is served directly by Loopbox at:

- `GET /v1/openapi.json`
- Discovery file includes `openapi_url`
- The OpenAPI document includes component schemas for project mutation requests, runtime responses, incident timelines, resource metrics, logs, requests, Doctor output, and service input.

## Incident timeline

`GET /v1/projects/{project}/incidents` returns an observe-only timeline for diagnostic handoff. Query parameters:

- `service` optionally filters to one service.
- `window` accepts `15m`, `1h`, `24h`, or `7d` and defaults to `1h`.
- `limit` caps returned events and is clamped by the server.

Runtime transitions are persisted in JSONL under `~/.config/loopbox/incident-events/` with fixed 7-day cleanup. Traffic failures, slow requests, resource pressure, resource unavailability, and log excerpts are synthesized from existing Loopbox stores at read time.

## Resource metrics

`GET /v1/projects/{project}/resources` returns persisted CPU, memory, process count, and container stat samples for active services. Query parameters:

- `service` optionally filters to one service.
- `window` accepts `15m`, `1h`, `24h`, or `7d` and defaults to `1h`.
- `limit` caps returned time-series samples and is clamped by the server.

Metrics collection is controlled by:

```toml
[global.resource_metrics]
enabled = true
sample_interval_secs = 5
retention_days = 7
max_storage_mb = 250
```

The interval is clamped to 2-60 seconds, retention to 1-90 days, and storage to 25-5,000 MB. When Docker or platform process stats are unavailable, samples remain best-effort and include `unavailable_reason` instead of failing the API response.

## Curl examples

```sh
BASE="$(jq -r '.base_url' ~/.config/loopbox/agent-api.json)"
AUTH_ENABLED="$(jq -r '.auth_enabled' ~/.config/loopbox/agent-api.json)"
TOKEN="$(cat ~/.config/loopbox/agent-api-token 2>/dev/null || true)"
AUTH="Authorization: Bearer ${TOKEN}"

curl -s "${BASE}/v1/health" | jq
curl -s "${BASE}/v1/openapi.json" | jq '.info'
curl -s -H "${AUTH}" "${BASE}/v1/doctor" | jq
if [ "${AUTH_ENABLED}" = "true" ]; then
  curl -s -H "${AUTH}" "${BASE}/v1/projects" | jq
else
  curl -s "${BASE}/v1/projects" | jq
fi

# Create a project (apply_system_setup is optional and defaults to false)
curl -s -X POST \
  -H "Content-Type: application/json" \
  -H "${AUTH}" \
  "${BASE}/v1/projects?apply_system_setup=false" \
  -d '{
    "name": "demo",
    "dir": "/path/to/demo",
    "ip": "127.0.0.30",
    "services": [
      {
        "name": "web",
        "runtime": "process",
        "command": "npm run dev",
        "workdir": "/path/to/demo",
        "ports": [{ "port": 5173, "protocol": "http1" }]
      }
    ]
  }
}' | jq

# Create a project with a container-backed Postgres service
curl -s -X POST \
  -H "Content-Type: application/json" \
  -H "${AUTH}" \
  "${BASE}/v1/projects?apply_system_setup=false" \
  -d '{
    "name": "db-demo",
    "dir": "/path/to/demo",
    "ip": "127.0.0.31",
    "services": [
      {
        "name": "postgres",
        "runtime": "container",
        "workdir": "/path/to/demo",
        "ports": [{ "port": 5432, "protocol": "tcp_passthrough" }],
        "container": {
          "image": "postgres:16-alpine",
          "env": ["POSTGRES_DB=app", "POSTGRES_PASSWORD=loopbox"],
          "volumes": ["db-demo-pgdata:/var/lib/postgresql/data"],
          "auto_remove": true
        }
      }
    ]
  }' | jq

# Send input to a process service only when runtime reports input_attached=true.
# terminal_attached means the in-app integrated terminal socket is available;
# terminal frames are intentionally not exposed over the Agent API in v1.
curl -s -H "${AUTH}" "${BASE}/v1/projects/demo/runtime" | jq '.services[] | {service, state, log_attached, input_attached, terminal_attached}'
curl -s -X POST \
  -H "Content-Type: application/json" \
  -H "${AUTH}" \
  "${BASE}/v1/projects/demo/services/web/input" \
  -d '{ "text": "r\n" }' | jq

# Inspect persisted resource utilization for the last hour
curl -s -H "${AUTH}" \
  "${BASE}/v1/projects/demo/resources?service=web&window=1h&limit=120" \
  | jq '{latest: .latest, sample_count: (.samples | length)}'

# Inspect recent incidents before drilling into logs, requests, or resources
curl -s -H "${AUTH}" \
  "${BASE}/v1/projects/demo/incidents?service=web&window=1h&limit=50" \
  | jq '.events[] | {severity, kind, summary, evidence_count: (.evidence | length)}'
```
