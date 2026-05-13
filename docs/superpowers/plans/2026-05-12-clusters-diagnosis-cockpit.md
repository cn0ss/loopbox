# Clusters Diagnosis Cockpit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Do not create git commits for this plan; the user explicitly requested no commits.

**Goal:** Turn Loopbox Clusters into a developer diagnosis cockpit with richer Kubernetes snapshots, topology, namespace focus, and safe debugging interactions.

**Architecture:** Keep Kubernetes command execution in `src/loopbox/kubernetes.rs`, with pure parser and topology helpers tested from JSON fixtures. Extend Agent API DTOs additively. Rework `src/app/pages/clusters.rs` into a compact Dioxus 0.7 tabbed workbench that consumes the richer snapshot without mutating Kubernetes objects.

**Tech Stack:** Rust 2021, Dioxus 0.7 signals/resources, serde JSON parsing, axum Agent API/OpenAPI, `kubectl` command integration.

---

## File Structure

- `src/loopbox/kubernetes.rs`: Owns Kubernetes snapshot structs, parser helpers, command wrappers, partial warning handling, namespace-aware snapshot assembly, and topology builder.
- `src/loopbox/agent_api.rs`: Owns DTO structs, DTO conversion, OpenAPI schema changes, and cluster detail query parameter handling.
- `src/loopbox/agent_api/routes.rs`: Wires namespace and metrics query parameters into cluster detail handlers if the route layer needs extraction.
- `src/app/pages/clusters.rs`: Owns Clusters page UI, selected namespace/view state, tabbed workbench, topology rendering, copy-command helpers, filtering helpers, and UI tests.
- `assets/main.css`: Owns cluster workbench, tabs, overview stats, topology, dense tables, event list, and responsive styling.
- `docs/agent-api.md`: Documents additive cluster response fields and query parameters.

## Task 1: Kubernetes Pod, Node, Ingress, EndpointSlice, And Event Parsers

**Files:**
- Modify: `src/loopbox/kubernetes.rs`

- [ ] **Step 1: Add snapshot structs and failing parser tests**

Add structs near the existing Kubernetes snapshot structs:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesNodeSnapshot {
    pub name: String,
    pub ready: bool,
    pub roles: Vec<String>,
    pub kubernetes_version: Option<String>,
    pub internal_ip: Option<String>,
    pub external_ip: Option<String>,
    pub allocatable_cpu: Option<String>,
    pub allocatable_memory: Option<String>,
    pub allocatable_pods: Option<String>,
    pub taints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesPodSnapshot {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub ready_containers: u64,
    pub total_containers: u64,
    pub restart_count: u64,
    pub owner_kind: Option<String>,
    pub owner_name: Option<String>,
    pub node_name: Option<String>,
    pub pod_ip: Option<String>,
    pub images: Vec<String>,
    pub warning_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesIngressSnapshot {
    pub name: String,
    pub namespace: String,
    pub class_name: Option<String>,
    pub hosts: Vec<String>,
    pub service_backends: Vec<String>,
    pub tls_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesEndpointSliceSnapshot {
    pub name: String,
    pub namespace: String,
    pub service_name: Option<String>,
    pub ready_endpoints: u64,
    pub total_endpoints: u64,
    pub addresses: Vec<String>,
    pub target_pods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesEventSnapshot {
    pub namespace: String,
    pub involved_kind: String,
    pub involved_name: String,
    pub event_type: String,
    pub reason: String,
    pub message: String,
    pub count: u64,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
}
```

Add tests in `mod tests`:

```rust
#[test]
fn parse_pod_snapshots_reads_readiness_restarts_owner_and_images() {
    let pods = super::parse_pod_snapshots(r#"{
      "items": [{
        "metadata": {
          "name": "api-7f9d",
          "namespace": "apps",
          "ownerReferences": [{ "kind": "ReplicaSet", "name": "api-7f9d" }]
        },
        "spec": {
          "nodeName": "node-a",
          "containers": [{ "image": "ghcr.io/acme/api:v1" }, { "image": "sidecar:v2" }]
        },
        "status": {
          "phase": "Running",
          "podIP": "10.1.0.15",
          "containerStatuses": [
            { "ready": true, "restartCount": 1 },
            { "ready": false, "restartCount": 2, "state": { "waiting": { "reason": "CrashLoopBackOff" } } }
          ]
        }
      }]
    }"#).expect("pods should parse");

    assert_eq!(pods.len(), 1);
    assert_eq!(pods[0].name, "api-7f9d");
    assert_eq!(pods[0].ready_containers, 1);
    assert_eq!(pods[0].total_containers, 2);
    assert_eq!(pods[0].restart_count, 3);
    assert_eq!(pods[0].owner_kind.as_deref(), Some("ReplicaSet"));
    assert_eq!(pods[0].node_name.as_deref(), Some("node-a"));
    assert_eq!(pods[0].warning_reason.as_deref(), Some("CrashLoopBackOff"));
}

#[test]
fn parse_node_snapshots_reads_ready_condition_addresses_and_allocatable() {
    let nodes = super::parse_node_snapshots(r#"{
      "items": [{
        "metadata": {
          "name": "node-a",
          "labels": { "node-role.kubernetes.io/control-plane": "" },
          "annotations": {}
        },
        "spec": { "taints": [{ "key": "node-role.kubernetes.io/control-plane", "effect": "NoSchedule" }] },
        "status": {
          "nodeInfo": { "kubeletVersion": "v1.30.0" },
          "allocatable": { "cpu": "4", "memory": "8123456Ki", "pods": "110" },
          "addresses": [
            { "type": "InternalIP", "address": "192.168.64.2" },
            { "type": "ExternalIP", "address": "203.0.113.10" }
          ],
          "conditions": [{ "type": "Ready", "status": "True" }]
        }
      }]
    }"#).expect("nodes should parse");

    assert_eq!(nodes.len(), 1);
    assert!(nodes[0].ready);
    assert_eq!(nodes[0].roles, vec!["control-plane"]);
    assert_eq!(nodes[0].internal_ip.as_deref(), Some("192.168.64.2"));
    assert_eq!(nodes[0].allocatable_cpu.as_deref(), Some("4"));
    assert_eq!(nodes[0].taints, vec!["node-role.kubernetes.io/control-plane=NoSchedule"]);
}
```

- [ ] **Step 2: Run parser tests and verify RED**

Run:

```sh
cargo test loopbox::kubernetes::tests::parse_pod_snapshots_reads_readiness_restarts_owner_and_images loopbox::kubernetes::tests::parse_node_snapshots_reads_ready_condition_addresses_and_allocatable
```

Expected: FAIL because parser functions do not exist.

- [ ] **Step 3: Implement minimal parser helpers**

Add pure helpers below `parse_service_snapshots`:

```rust
pub fn parse_pod_snapshots(stdout: &str) -> Result<Vec<KubernetesPodSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let name = metadata_name(item)?;
            let container_statuses = item
                .pointer("/status/containerStatuses")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let ready_containers = container_statuses
                .iter()
                .filter(|status| status.get("ready").and_then(Value::as_bool).unwrap_or(false))
                .count() as u64;
            let restart_count = container_statuses
                .iter()
                .filter_map(|status| status.get("restartCount").and_then(Value::as_u64))
                .sum();
            let warning_reason = container_statuses
                .iter()
                .find_map(container_waiting_reason);
            let images = item
                .pointer("/spec/containers")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(|container| container.get("image").and_then(Value::as_str).map(str::to_string))
                .collect::<Vec<_>>();
            let (owner_kind, owner_name) = first_owner_reference(item);
            Some(KubernetesPodSnapshot {
                name: name.to_string(),
                namespace: metadata_namespace(item).unwrap_or("default").to_string(),
                phase: item.pointer("/status/phase").and_then(Value::as_str).unwrap_or("Unknown").to_string(),
                ready_containers,
                total_containers: container_statuses.len() as u64,
                restart_count,
                owner_kind,
                owner_name,
                node_name: item.pointer("/spec/nodeName").and_then(Value::as_str).map(str::to_string),
                pod_ip: item.pointer("/status/podIP").and_then(Value::as_str).map(str::to_string),
                images,
                warning_reason,
            })
        })
        .collect())
}
```

Add helper functions:

```rust
fn container_waiting_reason(status: &Value) -> Option<String> {
    status
        .pointer("/state/waiting/reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn first_owner_reference(item: &Value) -> (Option<String>, Option<String>) {
    item.pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .and_then(|owners| owners.first())
        .map(|owner| {
            (
                owner.get("kind").and_then(Value::as_str).map(str::to_string),
                owner.get("name").and_then(Value::as_str).map(str::to_string),
            )
        })
        .unwrap_or((None, None))
}
```

Implement nodes:

```rust
pub fn parse_node_snapshots(stdout: &str) -> Result<Vec<KubernetesNodeSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let name = metadata_name(item)?;
            Some(KubernetesNodeSnapshot {
                name: name.to_string(),
                ready: node_ready(item),
                roles: node_roles(item),
                kubernetes_version: item.pointer("/status/nodeInfo/kubeletVersion").and_then(Value::as_str).map(str::to_string),
                internal_ip: node_address(item, "InternalIP"),
                external_ip: node_address(item, "ExternalIP"),
                allocatable_cpu: item.pointer("/status/allocatable/cpu").and_then(Value::as_str).map(str::to_string),
                allocatable_memory: item.pointer("/status/allocatable/memory").and_then(Value::as_str).map(str::to_string),
                allocatable_pods: item.pointer("/status/allocatable/pods").and_then(Value::as_str).map(str::to_string),
                taints: node_taints(item),
            })
        })
        .collect())
}
```

Add helpers for `node_ready`, `node_roles`, `node_address`, and `node_taints` that read standard Kubernetes JSON fields.

- [ ] **Step 4: Add ingress, EndpointSlice, and event parser tests**

Add tests for:

```rust
parse_ingress_snapshots_reads_hosts_tls_and_service_backends
parse_endpoint_slice_snapshots_reads_service_ready_endpoints_and_target_pods
parse_event_snapshots_reads_involved_object_reason_message_and_count
```

Use JSON fixtures that include one ingress rule to service `api`, one EndpointSlice labeled `kubernetes.io/service-name=api`, and one warning event for pod `api-7f9d`.

- [ ] **Step 5: Implement ingress, EndpointSlice, and event parsers**

Implement:

```rust
pub fn parse_ingress_snapshots(stdout: &str) -> Result<Vec<KubernetesIngressSnapshot>, String>
pub fn parse_endpoint_slice_snapshots(stdout: &str) -> Result<Vec<KubernetesEndpointSliceSnapshot>, String>
pub fn parse_event_snapshots(stdout: &str) -> Result<Vec<KubernetesEventSnapshot>, String>
```

Use structured JSON traversal only. Do not parse `kubectl` table output.

- [ ] **Step 6: Run parser tests and verify GREEN**

Run:

```sh
cargo test loopbox::kubernetes::tests::parse_pod_snapshots loopbox::kubernetes::tests::parse_node_snapshots loopbox::kubernetes::tests::parse_ingress_snapshots loopbox::kubernetes::tests::parse_endpoint_slice_snapshots loopbox::kubernetes::tests::parse_event_snapshots
```

Expected: PASS.

## Task 2: Namespace-Aware Partial Cluster Snapshot

**Files:**
- Modify: `src/loopbox/kubernetes.rs`

- [ ] **Step 1: Add snapshot extension fields**

Extend `KubernetesClusterSnapshot`:

```rust
pub selected_namespace: String,
pub nodes: Vec<KubernetesNodeSnapshot>,
pub pods: Vec<KubernetesPodSnapshot>,
pub ingresses: Vec<KubernetesIngressSnapshot>,
pub endpoint_slices: Vec<KubernetesEndpointSliceSnapshot>,
pub events: Vec<KubernetesEventSnapshot>,
pub warnings: Vec<String>,
```

Update every existing test fallback snapshot construction in `src/app/pages/clusters.rs` and `src/loopbox/kubernetes.rs` to fill these fields with defaults.

- [ ] **Step 2: Add namespace-aware public function**

Add:

```rust
pub fn cluster_snapshot_for_namespace(
    config: &LoopboxConfig,
    cluster_name: &str,
    namespace_override: Option<&str>,
) -> Result<KubernetesClusterSnapshot, String>
```

Make `cluster_snapshot(config, cluster_name)` delegate to this function with `None`.

- [ ] **Step 3: Implement partial warning collection**

Inside `cluster_snapshot_for_namespace`, use:

```rust
let mut warnings = Vec::<String>::new();
```

For each non-critical resource fetch, push `format!("{resource}: {err}")` into warnings and continue. Namespace list and basic cluster existence remain required.

- [ ] **Step 4: Fetch richer data**

Add read-only fetches:

```rust
get nodes -o json
get pods -o json
get ingresses -o json
get endpointslices -o json
get events -o json
```

Use namespace for pods/ingresses/endpointslices/events. Use no namespace for nodes.

- [ ] **Step 5: Run targeted tests**

Run:

```sh
cargo test loopbox::kubernetes::tests
```

Expected: PASS.

## Task 3: Kubernetes Topology Builder

**Files:**
- Modify: `src/loopbox/kubernetes.rs`

- [ ] **Step 1: Add topology structs**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesTopologySnapshot {
    pub nodes: Vec<KubernetesTopologyNode>,
    pub edges: Vec<KubernetesTopologyEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesTopologyNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub subtitle: String,
    pub status: String,
    pub badges: Vec<String>,
    pub column: usize,
    pub row: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesTopologyEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub label: String,
    pub status: String,
}
```

Add `pub topology: KubernetesTopologySnapshot` to `KubernetesClusterSnapshot`.

- [ ] **Step 2: Write topology test**

Create a test that builds topology from one deployment workload, one owned pod, one service with EndpointSlice target pod, and one ingress routing to the service. Assert nodes include namespace, workload, pod, service, ingress; assert edges include namespace containment, workload owns pod, service targets pod, ingress routes to service.

- [ ] **Step 3: Implement `build_kubernetes_topology`**

Add:

```rust
pub fn build_kubernetes_topology(snapshot: &KubernetesClusterSnapshot) -> KubernetesTopologySnapshot
```

Use deterministic IDs:

```rust
namespace:<name>
workload:<kind>:<namespace>:<name>
pod:<namespace>:<name>
service:<namespace>:<name>
ingress:<namespace>:<name>
```

Derive pod status from phase, warning reason, restart count, and readiness.

- [ ] **Step 4: Wire topology into snapshots**

In `cluster_snapshot_for_namespace`, build the snapshot first with `topology: KubernetesTopologySnapshot::default()`, then assign:

```rust
snapshot.topology = build_kubernetes_topology(&snapshot);
```

- [ ] **Step 5: Run tests**

Run:

```sh
cargo test loopbox::kubernetes::tests::build_kubernetes_topology
```

Expected: PASS.

## Task 4: Agent API Additive DTOs And Query Parameters

**Files:**
- Modify: `src/loopbox/agent_api.rs`
- Modify: `src/loopbox/agent_api/routes.rs`
- Modify: `docs/agent-api.md`

- [ ] **Step 1: Add DTO structs**

Add DTOs mirroring the new snapshots:

```rust
struct KubernetesNodeDto
struct KubernetesPodDto
struct KubernetesIngressDto
struct KubernetesEndpointSliceDto
struct KubernetesEventDto
struct KubernetesTopologyDto
struct KubernetesTopologyNodeDto
struct KubernetesTopologyEdgeDto
```

Add fields to `KubernetesClusterDto`: selected_namespace, nodes, pods, ingresses, endpoint_slices, events, warnings, topology.

- [ ] **Step 2: Update DTO conversion**

Extend `kubernetes_cluster_dto` and add conversion helpers for each new DTO. Keep existing fields unchanged.

- [ ] **Step 3: Add namespace query extraction**

In route handler for cluster detail, read optional query params:

```rust
#[derive(Debug, Deserialize)]
struct ClusterDetailQuery {
    namespace: Option<String>,
}
```

Call `cluster_snapshot_for_namespace(&config, &cluster_name, query.namespace.as_deref())`.

- [ ] **Step 4: Update OpenAPI schemas**

Add schemas for all new DTOs and add properties to `KubernetesClusterDto`.

- [ ] **Step 5: Update docs**

Document:

```md
GET /v1/clusters/{cluster}?namespace=apps
```

Mention that metrics are intentionally omitted in the first implementation and future optional metrics are best-effort.

- [ ] **Step 6: Run Agent API tests**

Run:

```sh
cargo test loopbox::agent_api::tests::cluster
```

Expected: PASS.

## Task 5: Cluster Workbench UI State And Tabs

**Files:**
- Modify: `src/app/pages/clusters.rs`
- Modify: `assets/main.css`

- [ ] **Step 1: Add UI enums and state**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterTab {
    Overview,
    Topology,
    Workloads,
    Services,
    Events,
    Config,
}
```

Add signals in `render_clusters_page`:

```rust
let mut active_tab = use_signal(|| ClusterTab::Overview);
let mut selected_namespace = use_signal(|| None::<String>);
let mut cluster_search = use_signal(String::new);
```

- [ ] **Step 2: Keep selected namespace synchronized**

When opening cluster details, set selected namespace to the cluster default namespace. Preserve selected namespace while refreshing the same cluster.

- [ ] **Step 3: Convert detail panel to tabbed workbench**

Replace the old single `ClusterDetailPanel` body with:

```rust
ClusterWorkbench {
    cluster,
    config,
    notice,
    runtime_tick,
    selected_cluster,
    edit_draft,
    active_tab,
    selected_namespace,
    cluster_search,
}
```

- [ ] **Step 4: Add tab bar CSS**

Add `.cluster-workbench`, `.cluster-tab-bar`, `.cluster-tab`, `.cluster-tab-active`, `.cluster-search-shell`, and responsive rules in `assets/main.css`.

- [ ] **Step 5: Run check**

Run:

```sh
cargo check
```

Expected: PASS.

## Task 6: Overview, Workloads, Services, Events, And Config Tabs

**Files:**
- Modify: `src/app/pages/clusters.rs`
- Modify: `assets/main.css`

- [ ] **Step 1: Implement overview tab**

Render compact stat tiles:

- Nodes ready / total
- Workloads healthy / total
- Pods ready / total
- Services with endpoints / total
- Warning events

- [ ] **Step 2: Implement workload table**

Show workload rows plus pod rows. Include status, namespace, ready, restarts, node, owner, images.

- [ ] **Step 3: Implement services table**

Show service type, cluster IP, ports, endpoint count, target pods, ingress routes.

- [ ] **Step 4: Implement events table**

Show type, reason, involved object, count, last timestamp, compact message.

- [ ] **Step 5: Move existing edit form into Config tab**

Preserve current update/delete/WireGuard behavior, but keep delete action visibly dangerous and not part of topology/workload tabs.

- [ ] **Step 6: Add helper tests**

Add tests for:

```rust
cluster_pod_ready_count_counts_ready_pods
cluster_warning_event_count_counts_warning_events
cluster_search_matches_workload_pod_service_and_event_text
```

- [ ] **Step 7: Run UI tests**

Run:

```sh
cargo test app::pages::clusters::tests
```

Expected: PASS.

## Task 7: Topology UI

**Files:**
- Modify: `src/app/pages/clusters.rs`
- Modify: `assets/main.css`

- [ ] **Step 1: Add selected topology node state**

Inside topology component, use:

```rust
let mut selected_node_id = use_signal(|| None::<String>);
```

- [ ] **Step 2: Render topology map**

Use the same pattern as sandbox topology: map shell, SVG edges, absolute-positioned buttons for nodes, right detail pane.

- [ ] **Step 3: Add topology toggles**

Add toggles for:

- Workloads
- Networking
- Pods
- Warnings

Node overlay remains hidden by default.

- [ ] **Step 4: Add copy summary action**

Generate a concise summary from topology nodes/edges and copy it through existing clipboard helper patterns in `clusters.rs`.

- [ ] **Step 5: Add topology CSS**

Add `.cluster-topology-*` classes mirroring the density and responsiveness of `.topology-*` without nesting cards.

- [ ] **Step 6: Run check**

Run:

```sh
cargo check
```

Expected: PASS.

## Task 8: Safe Debug Command Helpers

**Files:**
- Modify: `src/app/pages/clusters.rs`

- [ ] **Step 1: Add command builders**

Add helpers:

```rust
fn kubectl_base_args(cluster: &KubernetesClusterSnapshot, namespace: &str) -> Vec<String>
fn kubectl_get_command(cluster: &KubernetesClusterSnapshot, namespace: &str, kind: &str, name: &str) -> String
fn kubectl_logs_command(cluster: &KubernetesClusterSnapshot, namespace: &str, pod: &str) -> String
fn kubectl_port_forward_command(cluster: &KubernetesClusterSnapshot, namespace: &str, target: &str, local_port: u16, remote_port: u16) -> String
```

Commands must include `--context` and `--namespace`. They cannot include secret values.

- [ ] **Step 2: Add helper tests**

Test context and namespace inclusion, shell quoting for names with spaces, and service port-forward command format.

- [ ] **Step 3: Wire copy buttons**

Add copy buttons in detail panes for selected workload, pod, service, ingress, and event involved object.

- [ ] **Step 4: Run UI tests**

Run:

```sh
cargo test app::pages::clusters::tests
```

Expected: PASS.

## Task 9: Final Verification

**Files:**
- Modify as needed based on failures

- [ ] **Step 1: Run Kubernetes tests**

Run:

```sh
cargo test loopbox::kubernetes::tests
```

Expected: PASS.

- [ ] **Step 2: Run Agent API cluster tests**

Run:

```sh
cargo test loopbox::agent_api::tests::cluster
```

Expected: PASS.

- [ ] **Step 3: Run Clusters UI helper tests**

Run:

```sh
cargo test app::pages::clusters::tests
```

Expected: PASS.

- [ ] **Step 4: Run compile check**

Run:

```sh
cargo check
```

Expected: PASS.

- [ ] **Step 5: Completion audit**

Verify against the objective:

- More data shown: nodes, pods, ingresses, EndpointSlices/endpoints, events, warnings, richer workload/service fields.
- Better visualization: topology tab with nodes and edges.
- More interactions: namespace switch, search/filter, copy summaries, copy kubectl/log/port-forward commands, existing WireGuard controls preserved.
- Research applied: metrics are optional/deferred, EndpointSlices drive service relationships, Ingress is read-only, destructive Kubernetes mutations are excluded.
- No commits created.
