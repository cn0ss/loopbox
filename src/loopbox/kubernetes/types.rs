use crate::loopbox::KubernetesProvider;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KubernetesConnectivityState {
    NotConfigured,
    Active,
    Inactive,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesNamespaceSnapshot {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesWorkloadSnapshot {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub desired_replicas: Option<u64>,
    pub ready_replicas: Option<u64>,
    pub available_replicas: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesServiceSnapshot {
    pub name: String,
    pub namespace: String,
    pub service_type: String,
    pub cluster_ip: Option<String>,
    pub ports: Vec<String>,
    pub selector: Vec<String>,
    pub external_ips: Vec<String>,
    pub endpoint_count: u64,
    pub target_pods: Vec<String>,
    pub ingress_routes: Vec<String>,
}

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
    pub labels: Vec<String>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesClusterSnapshot {
    pub name: String,
    pub provider: KubernetesProvider,
    pub context: String,
    pub default_namespace: String,
    pub selected_namespace: String,
    pub connectivity: KubernetesConnectivityState,
    pub namespaces: Vec<KubernetesNamespaceSnapshot>,
    pub workloads: Vec<KubernetesWorkloadSnapshot>,
    pub services: Vec<KubernetesServiceSnapshot>,
    pub nodes: Vec<KubernetesNodeSnapshot>,
    pub pods: Vec<KubernetesPodSnapshot>,
    pub ingresses: Vec<KubernetesIngressSnapshot>,
    pub endpoint_slices: Vec<KubernetesEndpointSliceSnapshot>,
    pub events: Vec<KubernetesEventSnapshot>,
    pub topology: KubernetesTopologySnapshot,
    pub warnings: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesClusterDiscovery {
    pub name: String,
    pub provider: KubernetesProvider,
    pub kubeconfig_path: Option<String>,
    pub context: String,
    pub default_namespace: String,
    pub already_configured: bool,
    pub reachable: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesClusterImport {
    pub name: Option<String>,
    pub provider: KubernetesProvider,
    pub kubeconfig_path: Option<String>,
    pub context: String,
    pub default_namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KubectlInvocation {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) kubeconfig_env: Option<String>,
}
