# Contributing to Loopbox

Thanks for contributing.

## Contribution License

By submitting a contribution, you agree that your contribution is licensed under the project's [PolyForm Noncommercial License 1.0.0](LICENSE) and that the project author retains the right to offer commercial licenses for the combined work.

## Sign-Off Requirement (DCO)

All commits must be signed off.

Use:

```bash
git commit -s
```

The sign-off indicates your agreement with the Developer Certificate of Origin (DCO).

## Local Smoke Check

Before opening a pull request, run:

```bash
scripts/smoke-core-workflow.sh
scripts/smoke-agent-api-workflow.sh
```

The core smoke script checks formatting, runs the Rust test suite, runs clippy with warnings denied, and builds the app. The Agent API smoke script launches Loopbox in headless Agent API mode with isolated configs and verifies health, OpenAPI schemas, Doctor, project creation, runtime, logs, service input, stop, and auth-enabled access control.

If Docker is available locally, also run the optional Docker sandbox smoke:

```bash
scripts/smoke-docker-sandbox-port-reuse.sh
```

That script pulls/runs containers, skips cleanly when Docker is unavailable, and skips cleanly when the requested loopback aliases have not been installed yet. Run Loopbox System Setup or manually add the aliases before expecting a PASS. It is not part of the required PR gate.
