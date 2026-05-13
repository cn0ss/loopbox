use super::types::{
    KubernetesEndpointSliceSnapshot, KubernetesEventSnapshot, KubernetesIngressSnapshot,
    KubernetesNodeSnapshot, KubernetesPodSnapshot, KubernetesServiceSnapshot,
    KubernetesWorkloadSnapshot,
};
use serde_json::Value;

pub fn parse_namespace_names(stdout: &str) -> Result<Vec<String>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| metadata_name(item).map(str::to_string))
        .collect())
}

pub fn parse_workload_snapshots(
    kind: &str,
    stdout: &str,
) -> Result<Vec<KubernetesWorkloadSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let name = metadata_name(item)?;
            Some(KubernetesWorkloadSnapshot {
                kind: kind.to_string(),
                name: name.to_string(),
                namespace: metadata_namespace(item).unwrap_or("default").to_string(),
                desired_replicas: item
                    .pointer("/spec/replicas")
                    .and_then(Value::as_u64)
                    .or(Some(1)),
                ready_replicas: item
                    .pointer("/status/readyReplicas")
                    .and_then(Value::as_u64),
                available_replicas: item
                    .pointer("/status/availableReplicas")
                    .and_then(Value::as_u64),
            })
        })
        .collect())
}

pub fn parse_service_snapshots(stdout: &str) -> Result<Vec<KubernetesServiceSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let name = metadata_name(item)?;
            Some(KubernetesServiceSnapshot {
                name: name.to_string(),
                namespace: metadata_namespace(item).unwrap_or("default").to_string(),
                service_type: item
                    .pointer("/spec/type")
                    .and_then(Value::as_str)
                    .unwrap_or("ClusterIP")
                    .to_string(),
                cluster_ip: item
                    .pointer("/spec/clusterIP")
                    .and_then(Value::as_str)
                    .filter(|value| *value != "None")
                    .map(str::to_string),
                ports: service_port_labels(item),
                selector: label_pairs(item.pointer("/spec/selector")),
                external_ips: service_external_ips(item),
                endpoint_count: 0,
                target_pods: Vec::new(),
                ingress_routes: Vec::new(),
            })
        })
        .collect())
}

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
                kubernetes_version: item
                    .pointer("/status/nodeInfo/kubeletVersion")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                internal_ip: node_address(item, "InternalIP"),
                external_ip: node_address(item, "ExternalIP"),
                allocatable_cpu: item
                    .pointer("/status/allocatable/cpu")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                allocatable_memory: item
                    .pointer("/status/allocatable/memory")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                allocatable_pods: item
                    .pointer("/status/allocatable/pods")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                taints: node_taints(item),
            })
        })
        .collect())
}

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
                .filter(|status| {
                    status
                        .get("ready")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .count() as u64;
            let restart_count = container_statuses
                .iter()
                .filter_map(|status| status.get("restartCount").and_then(Value::as_u64))
                .sum();
            let warning_reason = container_statuses.iter().find_map(container_waiting_reason);
            let images = item
                .pointer("/spec/containers")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(|container| {
                    container
                        .get("image")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();
            let (owner_kind, owner_name) = first_owner_reference(item);
            Some(KubernetesPodSnapshot {
                name: name.to_string(),
                namespace: metadata_namespace(item).unwrap_or("default").to_string(),
                phase: item
                    .pointer("/status/phase")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string(),
                ready_containers,
                total_containers: container_statuses.len() as u64,
                restart_count,
                owner_kind,
                owner_name,
                node_name: item
                    .pointer("/spec/nodeName")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                pod_ip: item
                    .pointer("/status/podIP")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                images,
                labels: label_pairs(item.pointer("/metadata/labels")),
                warning_reason,
            })
        })
        .collect())
}

pub fn parse_ingress_snapshots(stdout: &str) -> Result<Vec<KubernetesIngressSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let name = metadata_name(item)?;
            Some(KubernetesIngressSnapshot {
                name: name.to_string(),
                namespace: metadata_namespace(item).unwrap_or("default").to_string(),
                class_name: item
                    .pointer("/spec/ingressClassName")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                hosts: ingress_hosts(item),
                service_backends: ingress_service_backends(item),
                tls_hosts: ingress_tls_hosts(item),
            })
        })
        .collect())
}

pub fn parse_endpoint_slice_snapshots(
    stdout: &str,
) -> Result<Vec<KubernetesEndpointSliceSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let name = metadata_name(item)?;
            let endpoints = item
                .get("endpoints")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let total_endpoints = endpoints.len() as u64;
            let ready_endpoints = endpoints
                .iter()
                .filter(|endpoint| {
                    endpoint
                        .pointer("/conditions/ready")
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
                })
                .count() as u64;
            Some(KubernetesEndpointSliceSnapshot {
                name: name.to_string(),
                namespace: metadata_namespace(item).unwrap_or("default").to_string(),
                service_name: item
                    .pointer("/metadata/labels/kubernetes.io~1service-name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                ready_endpoints,
                total_endpoints,
                addresses: endpoint_slice_addresses(&endpoints),
                target_pods: endpoint_slice_target_pods(&endpoints),
            })
        })
        .collect())
}

pub fn parse_event_snapshots(stdout: &str) -> Result<Vec<KubernetesEventSnapshot>, String> {
    let root = parse_json(stdout)?;
    Ok(root
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| {
            let involved_kind = item
                .pointer("/involvedObject/kind")
                .and_then(Value::as_str)
                .or_else(|| item.pointer("/regarding/kind").and_then(Value::as_str))?;
            let involved_name = item
                .pointer("/involvedObject/name")
                .and_then(Value::as_str)
                .or_else(|| item.pointer("/regarding/name").and_then(Value::as_str))?;
            Some(KubernetesEventSnapshot {
                namespace: metadata_namespace(item)
                    .or_else(|| {
                        item.pointer("/involvedObject/namespace")
                            .and_then(Value::as_str)
                    })
                    .unwrap_or("default")
                    .to_string(),
                involved_kind: involved_kind.to_string(),
                involved_name: involved_name.to_string(),
                event_type: item
                    .get("type")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("eventType").and_then(Value::as_str))
                    .unwrap_or("Normal")
                    .to_string(),
                reason: item
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string(),
                message: item
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("note").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string(),
                count: item.get("count").and_then(Value::as_u64).unwrap_or(1),
                first_timestamp: item
                    .get("firstTimestamp")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("eventTime").and_then(Value::as_str))
                    .map(str::to_string),
                last_timestamp: item
                    .get("lastTimestamp")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("eventTime").and_then(Value::as_str))
                    .map(str::to_string),
            })
        })
        .collect())
}

fn parse_json(stdout: &str) -> Result<Value, String> {
    serde_json::from_str(stdout).map_err(|err| format!("Invalid kubectl JSON: {err}"))
}

fn metadata_name(item: &Value) -> Option<&str> {
    item.pointer("/metadata/name").and_then(Value::as_str)
}

fn metadata_namespace(item: &Value) -> Option<&str> {
    item.pointer("/metadata/namespace").and_then(Value::as_str)
}

fn service_port_labels(item: &Value) -> Vec<String> {
    item.pointer("/spec/ports")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|port| {
            let port_number = port.get("port").and_then(Value::as_u64)?;
            let protocol = port
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("TCP");
            let name = port.get("name").and_then(Value::as_str).unwrap_or_default();
            let target = target_port_label(port.get("targetPort"))
                .unwrap_or_else(|| port_number.to_string());
            Some(if name.is_empty() {
                format!("{port_number}->{target}/{protocol}")
            } else {
                format!("{name}:{port_number}->{target}/{protocol}")
            })
        })
        .collect()
}

fn service_external_ips(item: &Value) -> Vec<String> {
    let mut values = item
        .pointer("/spec/externalIPs")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    values.extend(
        item.pointer("/status/loadBalancer/ingress")
            .and_then(Value::as_array)
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|entry| {
                entry
                    .get("ip")
                    .and_then(Value::as_str)
                    .or_else(|| entry.get("hostname").and_then(Value::as_str))
            })
            .map(str::to_string),
    );
    values.sort();
    values.dedup();
    values
}

fn label_pairs(value: Option<&Value>) -> Vec<String> {
    let mut labels = value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| {
                    let value = value.as_str().unwrap_or_default();
                    format!("{key}={value}")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    labels.sort();
    labels
}

fn node_ready(item: &Value) -> bool {
    item.pointer("/status/conditions")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .any(|condition| {
            condition.get("type").and_then(Value::as_str) == Some("Ready")
                && condition.get("status").and_then(Value::as_str) == Some("True")
        })
}

fn node_roles(item: &Value) -> Vec<String> {
    let mut roles = item
        .pointer("/metadata/labels")
        .and_then(Value::as_object)
        .map(|labels| {
            labels
                .keys()
                .filter_map(|key| key.strip_prefix("node-role.kubernetes.io/"))
                .filter(|role| !role.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if roles.is_empty() {
        roles.push("worker".to_string());
    }
    roles.sort();
    roles
}

fn node_address(item: &Value, address_type: &str) -> Option<String> {
    item.pointer("/status/addresses")
        .and_then(Value::as_array)?
        .iter()
        .find(|address| address.get("type").and_then(Value::as_str) == Some(address_type))
        .and_then(|address| address.get("address").and_then(Value::as_str))
        .map(str::to_string)
}

fn node_taints(item: &Value) -> Vec<String> {
    item.pointer("/spec/taints")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|taint| {
            let key = taint.get("key").and_then(Value::as_str)?;
            let effect = taint
                .get("effect")
                .and_then(Value::as_str)
                .unwrap_or("NoSchedule");
            Some(format!("{key}={effect}"))
        })
        .collect()
}

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
                owner
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                owner
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
        })
        .unwrap_or((None, None))
}

fn ingress_hosts(item: &Value) -> Vec<String> {
    let mut hosts = item
        .pointer("/spec/rules")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|rule| rule.get("host").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn ingress_service_backends(item: &Value) -> Vec<String> {
    let mut backends = Vec::new();
    if let Some(name) = item
        .pointer("/spec/defaultBackend/service/name")
        .and_then(Value::as_str)
    {
        backends.push(name.to_string());
    }
    for path in item
        .pointer("/spec/rules")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .flat_map(|rule| {
            rule.pointer("/http/paths")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
    {
        if let Some(name) = path
            .pointer("/backend/service/name")
            .and_then(Value::as_str)
        {
            backends.push(name.to_string());
        }
    }
    backends.sort();
    backends.dedup();
    backends
}

fn ingress_tls_hosts(item: &Value) -> Vec<String> {
    let mut hosts = item
        .pointer("/spec/tls")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .flat_map(|tls| {
            tls.get("hosts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|host| host.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn endpoint_slice_addresses(endpoints: &[Value]) -> Vec<String> {
    let mut values = endpoints
        .iter()
        .flat_map(|endpoint| {
            endpoint
                .get("addresses")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|address| address.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn endpoint_slice_target_pods(endpoints: &[Value]) -> Vec<String> {
    let mut pods = endpoints
        .iter()
        .filter_map(|endpoint| {
            let target = endpoint.get("targetRef")?;
            if target.get("kind").and_then(Value::as_str) == Some("Pod") {
                target
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    pods.sort();
    pods.dedup();
    pods
}

fn target_port_label(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        _ => None,
    }
}
