# Clusters Diagnosis Cockpit Design

Date: 2026-05-12

## Goal

Expand the Loopbox Clusters page from a narrow Kubernetes inventory view into a developer diagnosis cockpit. The page should show substantially more useful cluster data, visualize Kubernetes relationships, and support practical read-only debugging interactions while keeping the implementation aligned with Loopbox's local-first model.

This is not a replacement for Kubernetes Dashboard, Lens, or Grafana. Loopbox should help a developer understand "what is wrong with this cluster or namespace from my machine right now?" and then hand them precise evidence or commands.

## Current State

Loopbox already has a Kubernetes slice:

- Global config stores Kubernetes clusters and optional WireGuard tunnels.
- `src/loopbox/kubernetes.rs` shells out to `kubectl`, `wg`, and `wg-quick`.
- `KubernetesClusterSnapshot` currently includes namespaces, workloads, services, connectivity, and one `last_error`.
- Agent API exposes cluster list/detail, discovery/import, and WireGuard start/stop.
- `src/app/pages/clusters.rs` renders cluster cards, discovery/import, details/edit, namespace chips, workload rows, service rows, refresh, delete, and WireGuard actions.

The current UI is useful for setup, but too shallow for diagnosis. It does not show pods, nodes, ingress, recent events, selectors, endpoints, images, restart counts, age, conditions, namespace-focused drilldown, or a topology.

## Research Notes

Relevant external patterns:

- Kubernetes Dashboard groups views into admin overview, workloads, services, storage, config, logs, and object details.
- Lens uses a cluster catalog plus logical resource groups that help users navigate resource kinds.
- Octant emphasizes related-object graphs for understanding workload state.
- Speedscale-style cluster maps show namespaces, deployments, services, pods, and traffic/dependency edges.
- Grafana/Akuity-style infrastructure views are valuable for node/resource pressure, but Loopbox should treat metrics as optional because `kubectl top` depends on Metrics Server.

Relevant Kubernetes constraints:

- Services usually target pods by selector; EndpointSlices are the scalable API for service backends.
- Ingress maps HTTP(S) routes to services and is useful to visualize, but Gateway API support should be deferred.
- `kubectl top` is useful for spot checks only when Metrics Server is installed and should not block the cockpit.
- Events and field selectors are a good fit for read-only diagnosis.

## Product Shape

Use a tabbed cluster workbench:

- `Overview`: cluster health, namespace health, node summary, workload/service/pod counts, warning event count, optional resource availability.
- `Topology`: visual graph for namespace -> workload -> pod and ingress/service relationships.
- `Workloads`: dense table for deployments, statefulsets, daemonsets, replicasets, jobs, cronjobs, and pods.
- `Services`: services, selectors, endpoint counts, ingress routes, external IPs/hostnames, ports.
- `Events`: recent warning/normal events with filters by object kind, object name, reason, and namespace.
- `Config`: existing edit form, kubeconfig/context metadata, WireGuard controls, copyable kubectl commands.

The first screen should remain the Clusters page, not a landing page. When a cluster is selected, the detail panel becomes the workbench.

## Backend Model

Extend `KubernetesClusterSnapshot` with structured read-only data:

- `selected_namespace: String`
- `nodes: Vec<KubernetesNodeSnapshot>`
- `pods: Vec<KubernetesPodSnapshot>`
- `ingresses: Vec<KubernetesIngressSnapshot>`
- `events: Vec<KubernetesEventSnapshot>`
- `endpoint_slices: Vec<KubernetesEndpointSliceSnapshot>`
- `summary: KubernetesClusterSummary`
- `topology: KubernetesTopologySnapshot`
- `metrics: Option<KubernetesMetricsSnapshot>`
- `warnings: Vec<String>`

Keep existing fields for compatibility. Additive JSON/API changes should not break current Agent API consumers.

Suggested structs:

- Node: name, ready status, roles, Kubernetes version, internal/external IPs, allocatable CPU/memory/pods, age label, taints count.
- Pod: name, namespace, phase, ready containers, total containers, restart count, owner kind/name, node, pod IP, images, age label, warning reason.
- Workload: keep current replica fields and add unavailable replicas, updated replicas, labels/selectors, age label, condition summary.
- Service: keep current fields and add selector labels, external IPs, load balancer hostnames, endpoint count, target pod names, ingress route count.
- Ingress: name, namespace, class, hosts, rules, service backends, TLS hosts.
- EndpointSlice: name, namespace, service name, endpoint addresses, ready count, total count, target pod names.
- Event: namespace, involved kind/name, type, reason, message, count, first/last timestamp.
- Topology node: id, kind, label, subtitle, status, badges, column, row.
- Topology edge: from, to, kind, label, status.

All parsing should stay in pure helpers that accept JSON strings. Command wrappers should remain thin and return `Result<_, String>`.

## Snapshot Loading

Use namespace-focused loading:

- Cluster-wide: `kubectl get namespaces,nodes -o json`
- Namespace-scoped: pods, services, deployments, statefulsets, daemonsets, replicasets, jobs, cronjobs, ingresses, events, endpointslices
- Optional metrics: `kubectl top nodes` and `kubectl top pods --namespace <namespace>` when available

The UI should pass a selected namespace. If no namespace is selected, use the configured default namespace. If a namespace fetch fails, preserve the cluster shell and show warnings rather than hiding the cluster.

Metrics errors should produce non-fatal warnings such as "Metrics Server unavailable" and never become the primary `last_error` unless all core snapshot data is unavailable.

## Topology

Build topology from parsed Kubernetes objects:

- Namespace contains workloads, pods, services, ingresses.
- Workload owns pods through ownerReferences.
- Service targets pods by selector and/or EndpointSlices.
- Ingress routes to services.
- Optional node overlay connects pods to nodes when enabled.

Graph layout should reuse the style and interaction model of the existing sandbox topology:

- Toolbar with toggles for workloads, networking, nodes, warnings, and metrics.
- Map area with absolute-positioned nodes and SVG edges.
- Detail pane for the selected node.
- Copy summary and copy node detail actions.

Node statuses should be derived conservatively:

- Healthy: desired pods ready, no warning events.
- Warning: partial readiness, restarts, warning events, no endpoints, pending pods.
- Error: zero ready pods for a desired workload, crash loop-like reasons, failed pods.
- Unknown: insufficient data.

## Interactions

Initial interactions should be safe and read-only except existing WireGuard control:

- Refresh snapshot.
- Switch namespace.
- Select topology node.
- Filter/search workloads, services, and events.
- Copy kubectl command for object inspection.
- Copy object summary.
- Copy YAML/JSON command, not raw secret content.
- Open/copy pod logs command.
- Open/copy port-forward command for services/pods.
- Start/stop WireGuard only when already configured.

Defer destructive or high-risk actions:

- Delete/restart workloads.
- Edit Kubernetes objects.
- Apply YAML.
- Secret value display.
- Long-running managed port-forward sessions.

## UI Details

The design should feel like a dense operational tool, not a marketing page:

- Reuse existing Loopbox dark dashboard styling, compact buttons, chips, segmented controls, and detail panes.
- Do not use oversized cards or nested cards.
- Put the cluster workbench directly in the selected detail view.
- Use small badges and tables for scanability.
- Keep topology full-width within the workbench and pair it with a right-side detail pane on desktop.
- On narrow screens, stack topology above details and make tables single-column or horizontally scrollable only when necessary.

Use Dioxus 0.7 patterns only: `#[component]`, `Signal`, `use_signal`, `use_resource`, `use_memo`, and direct `rsx!` loops/conditionals. Do not use `cx`, `Scope`, or `use_state`.

## Agent API

Extend `KubernetesClusterDto` additively with the new snapshot data. Update OpenAPI schemas and `docs/agent-api.md`.

Add query parameters to cluster detail endpoints:

- `namespace`: overrides the configured default namespace for namespace-scoped data.
- `include_metrics`: defaults to false. The UI exposes metrics through an explicit toggle and passes true only when requested. The API should document metrics as optional and best-effort.

Agent output should allow diagnosis workflows like:

- Identify unhealthy workloads.
- Explain why a service has no endpoints.
- Find recent warning events for a pod or deployment.
- Produce safe kubectl commands for human review.

## Error Handling

Use partial snapshots:

- One failing resource kind should not blank the whole cluster.
- Each failed kubectl call should append a concise warning.
- `last_error` should represent the most important connectivity or authorization issue.
- UI warnings should be grouped and compact.

RBAC-limited clusters are expected. The UI should show "not authorized" warnings per resource kind and still render available data.

## Testing

Backend tests:

- Parse pods with readiness, restarts, owners, node, pod IP, images.
- Parse nodes with ready condition and allocatable fields.
- Parse ingresses with hosts/rules/backends.
- Parse EndpointSlices and map them to services/pods.
- Parse events with involved object, reason, type, message, and count.
- Build topology from workload, pod, service, ingress, and EndpointSlice fixtures.
- Verify partial snapshot warnings when one resource kind fails.

UI/helper tests:

- Summaries count healthy/warning/error workloads correctly.
- Namespace selection falls back to configured default.
- Search/filter helpers include workload, pod, service, ingress, event reason, and message text.
- Copy command helpers produce context, namespace, and kubeconfig-aware commands.

Verification:

- `cargo test loopbox::kubernetes::tests`
- `cargo test app::pages::clusters::tests`
- `cargo test loopbox::agent_api::tests::cluster`
- `cargo check`

## Phasing

Phase 1: Data model and parsers

- Add structs and parser tests for pods, nodes, ingresses, EndpointSlices, and events.
- Keep command execution read-only.

Phase 2: Snapshot composition

- Add namespace-aware cluster snapshot loading.
- Add partial warning handling.
- Add topology builder.

Phase 3: API

- Extend DTOs/OpenAPI/docs additively.
- Add namespace query parameter for details.

Phase 4: UI workbench

- Convert selected cluster detail into tabs.
- Add Overview, Topology, Workloads, Services, Events, Config.
- Preserve existing cluster discovery/import/edit/WireGuard behavior.

Phase 5: Verification and polish

- Run targeted tests and `cargo check`.
- Inspect responsive layout and dense table readability.

## Non-Goals

- Full Kubernetes admin replacement.
- Secret value inspection.
- Object mutation/edit/apply flows.
- Managed background port-forward sessions.
- Prometheus/Grafana integration.
- Gateway API support in the first pass.
- Historical metrics storage.

## Implementation Decisions

- Attempt metrics only behind an `include metrics` toggle.
- Copy log commands in the first pass rather than opening a Loopbox-managed log window.
- Hide the node overlay by default to keep the topology readable.
