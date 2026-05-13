use super::{
    KubernetesClusterSnapshot, KubernetesEventSnapshot, KubernetesPodSnapshot,
    KubernetesServiceSnapshot, KubernetesTopologyEdge, KubernetesTopologyNode,
    KubernetesTopologySnapshot, KubernetesWorkloadSnapshot,
};

pub fn build_kubernetes_topology(
    snapshot: &KubernetesClusterSnapshot,
) -> KubernetesTopologySnapshot {
    let namespace_id = topology_namespace_id(&snapshot.selected_namespace);
    let mut nodes = vec![KubernetesTopologyNode {
        id: namespace_id.clone(),
        kind: "namespace".to_string(),
        label: snapshot.selected_namespace.clone(),
        subtitle: format!(
            "{} pod(s), {} service(s)",
            snapshot.pods.len(),
            snapshot.services.len()
        ),
        status: if snapshot.warnings.is_empty() {
            "healthy".to_string()
        } else {
            "warning".to_string()
        },
        badges: vec![format!("{} events", warning_event_count(&snapshot.events))],
        column: 0,
        row: 0,
    }];
    let mut edges = Vec::new();

    for (index, workload) in snapshot.workloads.iter().enumerate() {
        let id = topology_workload_id(workload);
        nodes.push(KubernetesTopologyNode {
            id: id.clone(),
            kind: "workload".to_string(),
            label: workload.name.clone(),
            subtitle: format!("{} - {}", workload.kind, workload.namespace),
            status: workload_topology_status(workload),
            badges: vec![workload_ready_label(workload)],
            column: 1,
            row: index,
        });
        edges.push(KubernetesTopologyEdge {
            from: namespace_id.clone(),
            to: id,
            kind: "contains".to_string(),
            label: "contains".to_string(),
            status: "normal".to_string(),
        });
    }

    for (index, pod) in snapshot.pods.iter().enumerate() {
        let id = topology_pod_id(pod);
        nodes.push(KubernetesTopologyNode {
            id: id.clone(),
            kind: "pod".to_string(),
            label: pod.name.clone(),
            subtitle: pod.node_name.clone().unwrap_or_else(|| pod.phase.clone()),
            status: pod_topology_status(pod),
            badges: vec![
                format!("{}/{} ready", pod.ready_containers, pod.total_containers),
                format!("{} restarts", pod.restart_count),
            ],
            column: 2,
            row: index,
        });
        let mut connected = false;
        if let Some(workload) = pod_owner_workload(snapshot, pod) {
            edges.push(KubernetesTopologyEdge {
                from: topology_workload_id(workload),
                to: id.clone(),
                kind: "owns".to_string(),
                label: "owns".to_string(),
                status: pod_topology_status(pod),
            });
            connected = true;
        }
        if !connected {
            edges.push(KubernetesTopologyEdge {
                from: namespace_id.clone(),
                to: id,
                kind: "contains".to_string(),
                label: "contains".to_string(),
                status: pod_topology_status(pod),
            });
        }
    }

    for (index, service) in snapshot.services.iter().enumerate() {
        let id = topology_service_id(service);
        nodes.push(KubernetesTopologyNode {
            id: id.clone(),
            kind: "service".to_string(),
            label: service.name.clone(),
            subtitle: format!("{} - {}", service.service_type, service.namespace),
            status: if service.endpoint_count == 0 {
                "warning".to_string()
            } else {
                "healthy".to_string()
            },
            badges: vec![format!("{} endpoint(s)", service.endpoint_count)],
            column: 3,
            row: index,
        });
        edges.push(KubernetesTopologyEdge {
            from: namespace_id.clone(),
            to: id.clone(),
            kind: "contains".to_string(),
            label: "contains".to_string(),
            status: "normal".to_string(),
        });
        for pod_name in &service.target_pods {
            edges.push(KubernetesTopologyEdge {
                from: id.clone(),
                to: format!("pod:{}:{pod_name}", service.namespace),
                kind: "targets".to_string(),
                label: "targets".to_string(),
                status: "normal".to_string(),
            });
        }
    }

    for (index, ingress) in snapshot.ingresses.iter().enumerate() {
        let id = format!("ingress:{}:{}", ingress.namespace, ingress.name);
        nodes.push(KubernetesTopologyNode {
            id: id.clone(),
            kind: "ingress".to_string(),
            label: ingress.name.clone(),
            subtitle: ingress.hosts.join(", "),
            status: "healthy".to_string(),
            badges: ingress.service_backends.clone(),
            column: 4,
            row: index,
        });
        edges.push(KubernetesTopologyEdge {
            from: namespace_id.clone(),
            to: id.clone(),
            kind: "contains".to_string(),
            label: "contains".to_string(),
            status: "normal".to_string(),
        });
        for backend in &ingress.service_backends {
            edges.push(KubernetesTopologyEdge {
                from: id.clone(),
                to: format!("service:{}:{backend}", ingress.namespace),
                kind: "routes".to_string(),
                label: "routes".to_string(),
                status: "normal".to_string(),
            });
        }
    }

    KubernetesTopologySnapshot { nodes, edges }
}

fn topology_namespace_id(namespace: &str) -> String {
    format!("namespace:{namespace}")
}

fn topology_workload_id(workload: &KubernetesWorkloadSnapshot) -> String {
    format!(
        "workload:{}:{}:{}",
        workload.kind, workload.namespace, workload.name
    )
}

fn topology_pod_id(pod: &KubernetesPodSnapshot) -> String {
    format!("pod:{}:{}", pod.namespace, pod.name)
}

fn topology_service_id(service: &KubernetesServiceSnapshot) -> String {
    format!("service:{}:{}", service.namespace, service.name)
}

fn workload_ready_label(workload: &KubernetesWorkloadSnapshot) -> String {
    let ready = workload
        .ready_replicas
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let desired = workload
        .desired_replicas
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    format!("{ready}/{desired} ready")
}

fn workload_topology_status(workload: &KubernetesWorkloadSnapshot) -> String {
    match (workload.ready_replicas, workload.desired_replicas) {
        (Some(ready), Some(desired)) if desired > 0 && ready == 0 => "error".to_string(),
        (Some(ready), Some(desired)) if ready < desired => "warning".to_string(),
        _ => "healthy".to_string(),
    }
}

fn pod_topology_status(pod: &KubernetesPodSnapshot) -> String {
    if pod.phase == "Failed" || pod.warning_reason.is_some() {
        "error".to_string()
    } else if pod.phase != "Running" || pod.ready_containers < pod.total_containers {
        "warning".to_string()
    } else {
        "healthy".to_string()
    }
}

fn pod_owner_workload<'a>(
    snapshot: &'a KubernetesClusterSnapshot,
    pod: &KubernetesPodSnapshot,
) -> Option<&'a KubernetesWorkloadSnapshot> {
    let owner_name = pod.owner_name.as_deref()?;
    snapshot.workloads.iter().find(|workload| {
        workload.namespace == pod.namespace
            && (workload.name == owner_name || owner_name.starts_with(&workload.name))
    })
}

fn warning_event_count(events: &[KubernetesEventSnapshot]) -> usize {
    events
        .iter()
        .filter(|event| event.event_type.eq_ignore_ascii_case("warning"))
        .count()
}
