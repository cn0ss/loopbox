# Changelog

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
