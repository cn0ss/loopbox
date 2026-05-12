use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) struct DockerContainerState {
    pub(super) running: bool,
    pub(super) exit_code: Option<i32>,
}

pub(super) fn is_container_service(service: &ServiceConfig) -> bool {
    matches!(service.runtime, ServiceRuntimeKind::Container)
}

pub(super) fn start_container_service(
    project_name: &str,
    service: &ServiceConfig,
    bind_ip: &str,
    configured_ports: &[ServicePortConfig],
) -> Result<ServiceRuntimeSnapshot, String> {
    let container = service.container.as_ref().ok_or_else(|| {
        format!(
            "Service '{}' uses runtime 'container' but has no container configuration.",
            service.name
        )
    })?;
    if container.image.trim().is_empty() {
        return Err(format!(
            "Service '{}' uses runtime 'container' but container.image is empty.",
            service.name
        ));
    }

    let input = crate::loopbox::internal::runtime_container::StartContainerInput {
        project_name: project_name.to_string(),
        service_name: service.name.clone(),
        bind_ip: bind_ip.to_string(),
        ports: configured_ports.iter().map(|entry| entry.port).collect(),
        container: crate::loopbox::internal::runtime_container::ContainerSpec {
            image: container.image.clone(),
            args: container.args.clone(),
            env: container.env.clone(),
            volumes: container.volumes.clone(),
            auto_remove: container.auto_remove,
        },
    };
    crate::loopbox::internal::runtime_container::start_container(&input)?;

    Ok(ServiceRuntimeSnapshot {
        project: project_name.to_string(),
        service: service.name.clone(),
        state: ServiceRuntimeState::Starting,
        pid: None,
        started_at: Some(unix_timestamp(SystemTime::now())),
        exit_code: None,
        last_error: None,
    })
}

pub(super) fn container_runtime_status(
    project_name: &str,
    service: &ServiceConfig,
    bind_ip: &str,
    host: &str,
    key: &str,
) -> Result<ServiceRuntimeSnapshot, String> {
    let started_at_previous = {
        let store = runtime_store()
            .lock()
            .map_err(|_| "Runtime store lock poisoned.".to_string())?;
        store
            .history
            .get(key)
            .and_then(|snapshot| snapshot.started_at)
    };
    let started_at = started_at_previous.unwrap_or_else(|| unix_timestamp(SystemTime::now()));

    let container_name = runtime_container_name(project_name, &service.name);
    let snapshot = match docker_container_state(&container_name)? {
        None => ServiceRuntimeSnapshot {
            project: project_name.to_string(),
            service: service.name.clone(),
            state: ServiceRuntimeState::Stopped,
            pid: None,
            started_at: Some(started_at),
            exit_code: None,
            last_error: None,
        },
        Some(state) if state.running => {
            let runtime_targets = reachability_targets(bind_ip);
            let ports = service_ports(service);
            let elapsed = unix_timestamp_to_system_time(started_at)
                .elapsed()
                .unwrap_or_default()
                .as_secs();
            let mut runtime_state = if elapsed < STARTING_GRACE_PERIOD_SECS {
                ServiceRuntimeState::Starting
            } else {
                ServiceRuntimeState::Running
            };
            if elapsed >= STARTING_GRACE_PERIOD_SECS
                && !service_ports_healthy(&ports, &runtime_targets, host)
            {
                runtime_state = ServiceRuntimeState::Unhealthy;
            }

            ServiceRuntimeSnapshot {
                project: project_name.to_string(),
                service: service.name.clone(),
                state: runtime_state,
                pid: None,
                started_at: Some(started_at),
                exit_code: None,
                last_error: None,
            }
        }
        Some(state) => ServiceRuntimeSnapshot {
            project: project_name.to_string(),
            service: service.name.clone(),
            state: if state.exit_code.unwrap_or(0) == 0 {
                ServiceRuntimeState::Stopped
            } else {
                ServiceRuntimeState::Crashed
            },
            pid: None,
            started_at: Some(started_at),
            exit_code: state.exit_code,
            last_error: None,
        },
    };

    let mut store = runtime_store()
        .lock()
        .map_err(|_| "Runtime store lock poisoned.".to_string())?;
    upsert_runtime_history(&mut store, key.to_string(), snapshot.clone());
    Ok(snapshot)
}

pub(super) fn stop_container_service_if_present(
    project_name: &str,
    service_name: &str,
    previous: Option<&ServiceRuntimeSnapshot>,
) -> Result<Option<ServiceRuntimeSnapshot>, String> {
    let container_name = runtime_container_name(project_name, service_name);
    let state = match docker_container_state(&container_name) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    if state.is_none() {
        return Ok(None);
    }

    docker_remove_container(&container_name)?;
    Ok(Some(ServiceRuntimeSnapshot {
        project: project_name.to_string(),
        service: service_name.to_string(),
        state: ServiceRuntimeState::Stopped,
        pid: None,
        started_at: previous.and_then(|snapshot| snapshot.started_at),
        exit_code: None,
        last_error: None,
    }))
}

pub(super) fn runtime_container_name(project_name: &str, service_name: &str) -> String {
    crate::loopbox::internal::runtime_container::runtime_container_name(project_name, service_name)
}

pub(super) fn docker_container_state(name: &str) -> Result<Option<DockerContainerState>, String> {
    let state = crate::loopbox::internal::runtime_container::inspect_container(name)?;
    Ok(state.map(|state| DockerContainerState {
        running: state.running,
        exit_code: state.exit_code,
    }))
}

pub(super) fn docker_logs_tail_for_service(
    project_name: &str,
    service_name: &str,
    max_lines: usize,
) -> Result<Option<Vec<String>>, String> {
    let container_name = runtime_container_name(project_name, service_name);
    crate::loopbox::internal::runtime_container::logs_tail(&container_name, max_lines)
}

fn docker_remove_container(name: &str) -> Result<(), String> {
    crate::loopbox::internal::runtime_container::remove_container(name)
}
