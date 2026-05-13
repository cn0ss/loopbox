use crate::loopbox::{
    KubernetesClusterConfig, KubernetesProvider, WireGuardMode, WireGuardTunnelConfig,
};

fn test_cluster() -> KubernetesClusterConfig {
    KubernetesClusterConfig {
        name: "prod".to_string(),
        provider: KubernetesProvider::Remote,
        kubeconfig_path: Some("/tmp/prod-kubeconfig".to_string()),
        context: "prod-context".to_string(),
        default_namespace: "apps".to_string(),
        wireguard: Some(WireGuardTunnelConfig {
            name: "prod-wg".to_string(),
            mode: WireGuardMode::WgQuick,
            interface: Some("wg-prod0".to_string()),
            config_path: Some("/etc/wireguard/prod.conf".to_string()),
            required: true,
        }),
    }
}

#[test]
fn kubectl_invocation_includes_context_namespace_and_kubeconfig_env() {
    let invocation = super::kubectl_invocation(
        &test_cluster(),
        Some("observability"),
        &["get", "pods", "-o", "json"],
    );

    assert_eq!(invocation.program, "kubectl");
    assert_eq!(
        invocation.kubeconfig_env.as_deref(),
        Some("/tmp/prod-kubeconfig")
    );
    assert_eq!(
        invocation.args,
        vec![
            "--context",
            "prod-context",
            "--namespace",
            "observability",
            "get",
            "pods",
            "-o",
            "json"
        ]
    );
}

#[test]
fn parse_namespace_names_reads_kubectl_json_items() {
    let namespaces = super::parse_namespace_names(
        r#"{
          "items": [
            { "metadata": { "name": "default" } },
            { "metadata": { "name": "apps" } }
          ]
        }"#,
    )
    .expect("namespaces should parse");

    assert_eq!(namespaces, vec!["default", "apps"]);
}

#[test]
fn parse_workload_snapshots_reads_deployment_readiness() {
    let workloads = super::parse_workload_snapshots(
        "deployment",
        r#"{
          "items": [
            {
              "metadata": { "name": "api", "namespace": "apps" },
              "spec": { "replicas": 3 },
              "status": { "readyReplicas": 2, "availableReplicas": 1 }
            }
          ]
        }"#,
    )
    .expect("workloads should parse");

    assert_eq!(workloads.len(), 1);
    assert_eq!(workloads[0].kind, "deployment");
    assert_eq!(workloads[0].name, "api");
    assert_eq!(workloads[0].namespace, "apps");
    assert_eq!(workloads[0].desired_replicas, Some(3));
    assert_eq!(workloads[0].ready_replicas, Some(2));
    assert_eq!(workloads[0].available_replicas, Some(1));
}

#[test]
fn parse_service_snapshots_reads_cluster_ips_and_ports() {
    let services = super::parse_service_snapshots(
        r#"{
          "items": [
            {
              "metadata": { "name": "api", "namespace": "apps" },
              "spec": {
                "type": "ClusterIP",
                "clusterIP": "10.43.10.20",
                "ports": [
                  { "name": "http", "port": 80, "targetPort": 8080, "protocol": "TCP" }
                ]
              }
            }
          ]
        }"#,
    )
    .expect("services should parse");

    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "api");
    assert_eq!(services[0].namespace, "apps");
    assert_eq!(services[0].service_type, "ClusterIP");
    assert_eq!(services[0].cluster_ip.as_deref(), Some("10.43.10.20"));
    assert_eq!(services[0].ports, vec!["http:80->8080/TCP"]);
}

#[test]
fn parse_pod_snapshots_reads_readiness_restarts_owner_and_images() {
    let pods = super::parse_pod_snapshots(
        r#"{
          "items": [{
            "metadata": {
              "name": "api-7f9d",
              "namespace": "apps",
              "labels": { "app": "api" },
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
        }"#,
    )
    .expect("pods should parse");

    assert_eq!(pods.len(), 1);
    assert_eq!(pods[0].ready_containers, 1);
    assert_eq!(pods[0].total_containers, 2);
    assert_eq!(pods[0].restart_count, 3);
    assert_eq!(pods[0].owner_kind.as_deref(), Some("ReplicaSet"));
    assert_eq!(pods[0].node_name.as_deref(), Some("node-a"));
    assert_eq!(pods[0].warning_reason.as_deref(), Some("CrashLoopBackOff"));
    assert_eq!(pods[0].labels, vec!["app=api"]);
}

#[test]
fn parse_node_snapshots_reads_ready_condition_addresses_and_allocatable() {
    let nodes = super::parse_node_snapshots(
        r#"{
          "items": [{
            "metadata": {
              "name": "node-a",
              "labels": { "node-role.kubernetes.io/control-plane": "" }
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
        }"#,
    )
    .expect("nodes should parse");

    assert_eq!(nodes.len(), 1);
    assert!(nodes[0].ready);
    assert_eq!(nodes[0].roles, vec!["control-plane"]);
    assert_eq!(nodes[0].internal_ip.as_deref(), Some("192.168.64.2"));
    assert_eq!(nodes[0].allocatable_cpu.as_deref(), Some("4"));
    assert_eq!(
        nodes[0].taints,
        vec!["node-role.kubernetes.io/control-plane=NoSchedule"]
    );
}

#[test]
fn parse_ingress_endpoint_slice_and_events_feed_diagnosis() {
    let ingresses = super::parse_ingress_snapshots(
        r#"{
          "items": [{
            "metadata": { "name": "web", "namespace": "apps" },
            "spec": {
              "ingressClassName": "nginx",
              "tls": [{ "hosts": ["app.example.test"] }],
              "rules": [{
                "host": "app.example.test",
                "http": { "paths": [{ "backend": { "service": { "name": "api" } } }] }
              }]
            }
          }]
        }"#,
    )
    .expect("ingress should parse");
    assert_eq!(ingresses[0].hosts, vec!["app.example.test"]);
    assert_eq!(ingresses[0].service_backends, vec!["api"]);

    let slices = super::parse_endpoint_slice_snapshots(
        r#"{
          "items": [{
            "metadata": {
              "name": "api-abc",
              "namespace": "apps",
              "labels": { "kubernetes.io/service-name": "api" }
            },
            "endpoints": [{
              "addresses": ["10.1.0.15"],
              "conditions": { "ready": true },
              "targetRef": { "kind": "Pod", "name": "api-7f9d" }
            }]
          }]
        }"#,
    )
    .expect("endpoint slices should parse");
    assert_eq!(slices[0].service_name.as_deref(), Some("api"));
    assert_eq!(slices[0].ready_endpoints, 1);
    assert_eq!(slices[0].target_pods, vec!["api-7f9d"]);

    let events = super::parse_event_snapshots(
        r#"{
          "items": [{
            "metadata": { "namespace": "apps" },
            "involvedObject": { "kind": "Pod", "name": "api-7f9d", "namespace": "apps" },
            "type": "Warning",
            "reason": "BackOff",
            "message": "Back-off restarting failed container",
            "count": 4,
            "lastTimestamp": "2026-05-12T10:00:00Z"
          }]
        }"#,
    )
    .expect("events should parse");
    assert_eq!(events[0].event_type, "Warning");
    assert_eq!(events[0].reason, "BackOff");
    assert_eq!(events[0].count, 4);
}

#[test]
fn wireguard_show_output_detects_active_interface() {
    assert_eq!(
        super::wireguard_active_from_show_output(true, "interface: wg-prod0\n", ""),
        Ok(true)
    );
    assert_eq!(
        super::wireguard_active_from_show_output(false, "", "Unable to access interface"),
        Ok(false)
    );
}

#[test]
fn context_name_parser_trims_blank_lines() {
    assert_eq!(
        super::parse_context_names("\nkind-loopbox\n  prod-eu  \n\n"),
        vec!["kind-loopbox", "prod-eu"]
    );
}

#[test]
fn import_kubernetes_clusters_adds_new_contexts_and_skips_duplicates() {
    let mut config = crate::loopbox::LoopboxConfig::default();
    config
        .global
        .kubernetes
        .clusters
        .push(KubernetesClusterConfig {
            name: "existing".to_string(),
            provider: KubernetesProvider::KubeconfigContext,
            kubeconfig_path: Some("/tmp/kube".to_string()),
            context: "prod".to_string(),
            default_namespace: "apps".to_string(),
            wireguard: None,
        });

    let added = super::import_kubernetes_clusters(
        &mut config,
        &[
            super::KubernetesClusterImport {
                name: None,
                provider: KubernetesProvider::KubeconfigContext,
                kubeconfig_path: Some("/tmp/kube".to_string()),
                context: "prod".to_string(),
                default_namespace: Some("apps".to_string()),
            },
            super::KubernetesClusterImport {
                name: None,
                provider: KubernetesProvider::KubeconfigContext,
                kubeconfig_path: Some("/tmp/local".to_string()),
                context: "kind-loopbox".to_string(),
                default_namespace: None,
            },
        ],
    )
    .expect("import should succeed");

    assert_eq!(added, 1);
    assert_eq!(config.global.kubernetes.clusters.len(), 2);
    let imported = &config.global.kubernetes.clusters[1];
    assert_eq!(imported.name, "kind-loopbox");
    assert_eq!(imported.context, "kind-loopbox");
    assert_eq!(imported.default_namespace, "default");
    assert_eq!(imported.kubeconfig_path.as_deref(), Some("/tmp/local"));
}
