# Kubernetes Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a first complete Kubernetes control-plane slice to Loopbox: configure local/remote clusters, model optional WireGuard connectivity, inspect cluster/workload status, expose Doctor and Agent API surfaces, and add a Dioxus clusters page.

**Architecture:** Kubernetes is a new domain beside the existing sandbox runtime. `LoopboxConfig` owns a global cluster registry; a focused `loopbox::kubernetes` module shells out to `kubectl` and WireGuard commands through small parsing/status helpers; UI and Agent API consume typed snapshots rather than parsing command output.

**Tech Stack:** Rust 2021, Dioxus 0.7, serde/TOML, axum Agent API, command-line integrations for `kubectl`, `wg`, and `wg-quick`.

---

### Task 1: Config Model And Normalization

**Files:**
- Modify: `src/loopbox.rs`
- Modify: `src/loopbox/config.rs`
- Test: `src/loopbox/config/tests.rs`

- [ ] **Step 1: Write failing config tests**

Add tests covering default empty Kubernetes settings, TOML loading for a remote cluster with WireGuard, and normalization that trims names/context/namespace and drops invalid empty clusters.

- [ ] **Step 2: Run config tests and verify RED**

Run: `cargo test loopbox::config::tests::kubernetes`
Expected: FAIL because `global.kubernetes` and related types do not exist.

- [ ] **Step 3: Add config types**

Add `KubernetesSettings`, `KubernetesClusterConfig`, `KubernetesProvider`, `WireGuardTunnelConfig`, and `WireGuardMode` to `src/loopbox.rs`, with serde defaults and `Default` impls.

- [ ] **Step 4: Normalize Kubernetes settings**

Extend `normalize_config` in `src/loopbox/config.rs` so cluster names are lowercased, namespaces default to `default`, provider defaults to `kubeconfig_context`, and empty cluster names/contexts are removed.

- [ ] **Step 5: Run config tests and verify GREEN**

Run: `cargo test loopbox::config::tests::kubernetes`
Expected: PASS.

### Task 2: Kubernetes Command Snapshot Module

**Files:**
- Create: `src/loopbox/kubernetes.rs`
- Modify: `src/loopbox.rs`
- Test: `src/loopbox/kubernetes.rs`

- [ ] **Step 1: Write failing parser/status tests**

Add unit tests for parsing `kubectl get namespaces -o json`, parsing deployment/service JSON, WireGuard status decisions, and command argument construction.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test loopbox::kubernetes::tests`
Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement typed snapshots**

Create cluster, namespace, workload, service, and connectivity snapshot structs. Implement pure parser helpers that accept command stdout strings.

- [ ] **Step 4: Implement command wrappers**

Add functions that run `kubectl --context <context> --namespace <namespace> ...`, `wg show <interface>`, and `wg-quick up/down <config_path>` where configured. Keep every public function returning `Result<_, String>`.

- [ ] **Step 5: Run tests and verify GREEN**

Run: `cargo test loopbox::kubernetes::tests`
Expected: PASS.

### Task 3: Doctor Integration

**Files:**
- Modify: `src/loopbox/doctor.rs`
- Test: `src/loopbox/config/tests.rs` or focused doctor tests if a better local test module exists

- [ ] **Step 1: Write failing Doctor tests**

Add tests that a required WireGuard tunnel produces a warning when inactive and that a configured cluster produces kubectl/context checks.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test loopbox::doctor`
Expected: FAIL because Kubernetes Doctor checks are missing.

- [ ] **Step 3: Add Kubernetes Doctor checks**

Doctor should report missing `kubectl`, unreadable kubeconfig paths, unreachable contexts, missing namespaces, and inactive required WireGuard tunnels. Avoid mutating cluster state from Doctor.

- [ ] **Step 4: Run tests and verify GREEN**

Run: `cargo test loopbox::doctor loopbox::config::tests::kubernetes`
Expected: PASS.

### Task 4: Agent API

**Files:**
- Modify: `src/loopbox/agent_api.rs`
- Modify: `src/loopbox/agent_api/routes.rs`
- Modify: `docs/agent-api.md`
- Test: `src/loopbox/agent_api.rs`

- [ ] **Step 1: Write failing Agent API tests**

Add tests for DTO conversion and OpenAPI path presence for `/v1/clusters` and `/v1/clusters/{cluster}`.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test loopbox::agent_api::tests::cluster`
Expected: FAIL because cluster routes and DTOs do not exist.

- [ ] **Step 3: Add read endpoints**

Add `GET /v1/clusters`, `GET /v1/clusters/{cluster}`, `POST /v1/clusters/{cluster}/wireguard/start`, and `POST /v1/clusters/{cluster}/wireguard/stop`.

- [ ] **Step 4: Document endpoints**

Update `docs/agent-api.md` with endpoint list and curl examples.

- [ ] **Step 5: Run tests and verify GREEN**

Run: `cargo test loopbox::agent_api::tests::cluster`
Expected: PASS.

### Task 5: Dioxus UI

**Files:**
- Modify: `src/app/models.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/sidebar.rs`
- Create: `src/app/pages/clusters.rs`
- Modify: `src/app/pages/mod.rs`
- Modify: `assets/main.css`

- [ ] **Step 1: Add page model and route wiring**

Add `Page::Clusters`, sidebar entry, and page rendering call.

- [ ] **Step 2: Implement clusters page**

Render configured clusters, provider/context/namespace, WireGuard status, namespaces, workloads, and services. Include start/stop buttons for configured WireGuard tunnels and refresh through Dioxus `use_resource`.

- [ ] **Step 3: Add styling**

Use existing dense dashboard styling patterns. Keep controls compact and consistent with Runtime/System pages.

- [ ] **Step 4: Run compile check**

Run: `cargo check`
Expected: PASS.

### Task 6: Final Verification

**Files:**
- Modify as needed based on failures

- [ ] **Step 1: Run targeted tests**

Run: `cargo test loopbox::config::tests::kubernetes loopbox::kubernetes::tests loopbox::agent_api::tests::cluster`
Expected: PASS.

- [ ] **Step 2: Run full Rust check**

Run: `cargo check`
Expected: PASS.

- [ ] **Step 3: Completion audit**

Confirm Kubernetes config, local/remote kubeconfig contexts, WireGuard status/control, Doctor, Agent API, and UI are all represented by code and tests or compile verification.
