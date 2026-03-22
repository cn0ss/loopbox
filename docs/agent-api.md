# Loopbox Agent API (Local)

Loopbox now exposes a local HTTP API for agent tools.

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
- `GET /v1/projects`
- `POST /v1/projects?apply_system_setup={true|false}`
- `GET /v1/projects/{project}`
- `PUT /v1/projects/{project}?apply_system_setup={true|false}`
- `GET /v1/projects/{project}/runtime`
- `GET /v1/projects/{project}/logs?service={service}&limit={n}`
- `GET /v1/projects/{project}/requests?service={service?}&limit={n}`
- `POST /v1/projects/{project}/start`
- `POST /v1/projects/{project}/stop`
- `POST /v1/projects/{project}/restart`
- `POST /v1/projects/{project}/services/{service}/start`
- `POST /v1/projects/{project}/services/{service}/stop`
- `POST /v1/projects/{project}/services/{service}/restart`

OpenAPI is served directly by Loopbox at:

- `GET /v1/openapi.json`
- Discovery file includes `openapi_url`

## Curl examples

```sh
BASE="$(jq -r '.base_url' ~/.config/loopbox/agent-api.json)"
AUTH_ENABLED="$(jq -r '.auth_enabled' ~/.config/loopbox/agent-api.json)"
TOKEN="$(cat ~/.config/loopbox/agent-api-token 2>/dev/null || true)"
AUTH="Authorization: Bearer ${TOKEN}"

curl -s "${BASE}/v1/health" | jq
curl -s "${BASE}/v1/openapi.json" | jq '.info'
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
  }' | jq
```
