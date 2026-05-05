use super::wizard::{sanitize_identifier, unique_service_name};
use super::*;

pub(super) fn build_compose_discovered_services(
    project_dir: &str,
    compose_services: &[loopbox::ComposeServiceSuggestion],
) -> Vec<ServiceEntry> {
    let docker_management_enabled = true;
    let mut used_names = HashSet::new();
    let mut services = Vec::new();

    for compose_service in compose_services {
        let base_name = sanitize_identifier(&compose_service.service_name);
        let service_name = unique_service_name(base_name, &mut used_names);

        let mut entry = wizard_blank_service_entry_base();
        entry.name = service_name.clone();
        entry.workdir = project_dir.to_string();
        entry.env_files = compose_service.env_files.join(",");
        entry.depends_on = compose_service.depends_on.join(",");
        entry.autostart = false;

        let mut port_rows = compose_service
            .ports
            .iter()
            .filter(|port| !port.protocol.eq_ignore_ascii_case("udp"))
            .map(|port| ServicePortEntry {
                port: port.published_port.to_string(),
                protocol: compose_proxy_protocol(&service_name, port.published_port).to_string(),
                health_path: String::new(),
            })
            .collect::<Vec<_>>();
        if port_rows.is_empty() {
            port_rows.push(blank_service_port_entry());
        }
        entry.ports = port_rows;
        sync_service_entry_primary_port(&mut entry);

        let image = compose_service
            .image
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();

        if docker_management_enabled && !image.is_empty() {
            entry.runtime = "container".to_string();
            entry.command.clear();
            entry.container_image = image;
            entry.container_args = compose_service.command.join("\n");
            entry.container_env = compose_service.env.join("\n");
            entry.container_volumes = compose_service.volumes.join("\n");
            entry.container_auto_remove = true;
        } else {
            entry.runtime = "process".to_string();
            entry.command = compose_service_process_command(compose_service);
            entry.container_image.clear();
            entry.container_args.clear();
            entry.container_env.clear();
            entry.container_volumes.clear();
            entry.container_auto_remove = false;
        }

        services.push(entry);
    }

    services
}

fn compose_service_process_command(service: &loopbox::ComposeServiceSuggestion) -> String {
    let service_name = service.service_name.trim();
    if service_name.is_empty() {
        return "docker compose up".to_string();
    }
    if service.uses_build {
        format!("docker compose up --build {service_name}")
    } else {
        format!("docker compose up {service_name}")
    }
}

fn compose_proxy_protocol(service_name: &str, port: u16) -> &'static str {
    let lowered = service_name.to_ascii_lowercase();
    if lowered.contains("grpc") || matches!(port, 50051 | 50052 | 6565) {
        return "grpc_h2c";
    }
    if lowered.contains("web")
        || lowered.contains("front")
        || lowered.contains("ui")
        || lowered.contains("api")
        || lowered.contains("gateway")
        || matches!(
            port,
            80 | 81
                | 3000
                | 3001
                | 4173
                | 4200
                | 5000
                | 5173
                | 8000
                | 8080
                | 8081
                | 8888
                | 9000
                | 1885
                | 1886
                | 1887
        )
    {
        return "http1";
    }
    "tcp_passthrough"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_import_maps_container_and_process_services() {
        let services = build_compose_discovered_services(
            "/tmp/demo",
            &[
                loopbox::ComposeServiceSuggestion {
                    service_name: "postgres".to_string(),
                    image: Some("postgres:16-alpine".to_string()),
                    command: vec![
                        "postgres".to_string(),
                        "-c".to_string(),
                        "fsync=off".to_string(),
                    ],
                    env: vec![
                        "POSTGRES_DB=app".to_string(),
                        "POSTGRES_PASSWORD=secret".to_string(),
                    ],
                    env_files: vec![".env.db".to_string()],
                    volumes: vec!["pgdata:/var/lib/postgresql/data".to_string()],
                    depends_on: vec![],
                    ports: vec![loopbox::ComposePortSuggestion {
                        published_port: 5432,
                        protocol: "tcp".to_string(),
                    }],
                    uses_build: false,
                },
                loopbox::ComposeServiceSuggestion {
                    service_name: "api".to_string(),
                    image: None,
                    command: vec![],
                    env: vec![],
                    env_files: vec![".env".to_string()],
                    volumes: vec![".:/app".to_string()],
                    depends_on: vec!["postgres".to_string()],
                    ports: vec![loopbox::ComposePortSuggestion {
                        published_port: 8080,
                        protocol: "tcp".to_string(),
                    }],
                    uses_build: true,
                },
            ],
        );

        assert_eq!(services.len(), 2);

        let db = &services[0];
        assert_eq!(db.name, "postgres");
        assert_eq!(db.runtime, "container");
        assert_eq!(db.command, "");
        assert_eq!(db.workdir, "/tmp/demo");
        assert_eq!(db.env_files, ".env.db");
        assert_eq!(db.container_image, "postgres:16-alpine");
        assert_eq!(db.container_args, "postgres\n-c\nfsync=off");
        assert_eq!(
            db.container_env,
            "POSTGRES_DB=app\nPOSTGRES_PASSWORD=secret"
        );
        assert_eq!(db.container_volumes, "pgdata:/var/lib/postgresql/data");
        assert_eq!(db.ports[0].port, "5432");
        assert_eq!(db.ports[0].protocol, "tcp_passthrough");

        let api = &services[1];
        assert_eq!(api.name, "api");
        assert_eq!(api.runtime, "process");
        assert_eq!(api.command, "docker compose up --build api");
        assert_eq!(api.env_files, ".env");
        assert_eq!(api.depends_on, "postgres");
        assert_eq!(api.container_image, "");
        assert_eq!(api.ports[0].port, "8080");
        assert_eq!(api.ports[0].protocol, "http1");
    }
}
