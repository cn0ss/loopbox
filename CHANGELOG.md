# Changelog

## v0.3.5 - 2026-05-13

### Added
- Added Kubernetes cluster management with kubeconfig discovery/import, namespace/workload/service inspection, topology summaries, and optional WireGuard controls.

### Changed
- Split the app stylesheet into focused asset files and expanded runtime, diagnostics, sandbox, and Agent API views for cluster-aware workflows.
- Documented Kubernetes Agent API endpoints and configurable health-check intervals.

## v0.3.4 - 2026-05-12

### Fixed
- Fixed stale WebView CSS caching that could leave the new Topology tab without its dedicated styling after an in-app update.
- Cleaned stale macOS app bundle assets before release builds so old hashed CSS files are not carried into signed update artifacts.

## v0.3.3 - 2026-05-12

### Added
- Added a per-sandbox Topology tab that maps services, declared dependencies, HTTP ingress, proxy endpoints, runtime health, incidents, resource metrics, and recent traffic in one read-only operational view.
- Added Incident Timeline diagnostics that correlate runtime transitions, traffic failures, slow requests, resource pressure, and log excerpts.
- Added Diagnosis Sessions for starting agent-assisted investigations from sandboxes, runtime alerts, and incident events, with durable reports and resolution tracking.

## v0.3.2 - 2026-05-05

### Added
- Added persisted service resource metrics for CPU, memory, process counts, and container stats, including Runtime UI summaries, per-project trend cards, Settings controls, and Agent API `/resources` output.
- Added a headless Agent API server mode for CI/smoke validation without launching the desktop UI.
- Added Agent API Doctor and service input coverage to the OpenAPI schema and smoke workflow, including auth-enabled checks.
- Added Agent API resource metrics coverage to the smoke workflow.
- Added container runtime hardening around Docker availability, command construction, stopped container replacement, logs, and removal handling.

### Changed
- Resource metric disk reads now use bounded, newest-first/window-filtered readers instead of loading every persisted sample on hot paths.
- Agent API project mutations now skip reverse proxy sync when the config has no routable service ports or proxy endpoints.
- Docker sandbox port-reuse smoke now skips cleanly when required loopback aliases are missing.
- Sandbox preflight now flags duplicate service ports on the target sandbox IP before creation.
- Sandbox preflight now validates project config without writing agent guidance files.
- Container services no longer show process terminal/run controls in the sandbox detail runtime actions.

## v0.3.1 - 2026-05-05

### Changed
- Removed runtime license activation, Polar integration, and edition-based feature gates.
- Folded public release tooling into this repo and removed `loopbox-ee`/overlay release assumptions.
- Renamed stale paid/EE UI modules to neutral feature names.
- Clarified platform support as macOS primary with experimental Windows networking/runtime support.

## v1.0.5 - 2026-03-01

### Added
- Runtime terminal controls for interactive services, including attach/send-input flows for long-running TTY tools.

### Changed
- Runtime lifecycle tracking and recovery were tightened so detached/stale service entries are reconciled more reliably.
- Service health and status handling was refined around running/unhealthy transitions.

### Fixed
- Service command input now normalizes smart dash variants (`—`, `–`, `−`) to standard CLI flags (`--`, `-`) in wizard/detail UI and config parsing.
- Service log rendering now strips terminal control/escape sequences to avoid unreadable glyph artifacts in the UI.
- Runtime signal safety guards now reject invalid `pid`/`pgid` `0` targets, and `pid=0` registry entries are pruned.

## v0.1.3 - 2026-02-25

### Added
- Agent API audit tab to inspect local Agent API request/response records, including headers, body snapshots, status, and latency.

### Changed
- Agent API scope is now explicitly documented as create/update capable for projects (delete remains intentionally unsupported).
- Agent API audit is included in the public app feature implementation.

## v0.1.2 - 2026-02-24

### Changed
- Refactored large modules into focused submodules and extracted inline tests into dedicated files.
- Improved UI input responsiveness by reducing hot-path recomputation work.
- Clarified public licensing and all-feature availability.
- Refreshed docs/UI copy around feature availability.

## v0.1.1 - 2026-02-23

### Added
- Initial public release baseline.
