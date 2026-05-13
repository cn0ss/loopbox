use super::*;
use crate::loopbox::ServiceRuntimeKind;

fn parse_service_runtime(raw: &str) -> ServiceRuntimeKind {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "container" => ServiceRuntimeKind::Container,
        _ => ServiceRuntimeKind::Process,
    }
}

fn service_runtime_value(runtime: ServiceRuntimeKind) -> &'static str {
    match runtime {
        ServiceRuntimeKind::Process => "process",
        ServiceRuntimeKind::Container => "container",
    }
}

fn service_entry_runtime(entry: &ServiceEntry) -> ServiceRuntimeKind {
    parse_service_runtime(&entry.runtime)
}

fn service_entry_requires_command(entry: &ServiceEntry) -> bool {
    !matches!(service_entry_runtime(entry), ServiceRuntimeKind::Container)
}

fn service_entry_is_ready(entry: &ServiceEntry) -> bool {
    if service_entry_requires_command(entry) {
        !entry.command.trim().is_empty()
    } else {
        !entry.container_image.trim().is_empty()
    }
}

pub(super) fn wizard_service_entry_is_ready(entry: &ServiceEntry) -> bool {
    service_entry_is_ready(entry)
}

pub(super) fn wizard_service_step_subtitle() -> &'static str {
    "Edit services. For process runtime, assign commands. For container runtime, set image and optional Docker options."
}

pub(super) fn wizard_service_requirement_hint() -> &'static str {
    "Each named service needs a command (process) or image (container)."
}

pub(super) fn wizard_blank_service_entry_base() -> ServiceEntry {
    ServiceEntry {
        name: String::new(),
        ports: vec![blank_service_port_entry()],
        port: String::new(),
        protocol: "http1".to_string(),
        runtime: "process".to_string(),
        command: String::new(),
        workdir: String::new(),
        env_files: String::new(),
        depends_on: String::new(),
        autostart: false,
        health_path: String::new(),
        container_image: String::new(),
        container_args: String::new(),
        container_env: String::new(),
        container_volumes: String::new(),
        container_auto_remove: true,
    }
}

pub(super) fn wizard_discovered_service_entry(
    name: String,
    port: Option<u16>,
    suggestion: &loopbox::DiscoverySuggestion,
) -> ServiceEntry {
    ServiceEntry {
        name,
        ports: vec![ServicePortEntry {
            port: port.map(|value| value.to_string()).unwrap_or_default(),
            protocol: "http1".to_string(),
            health_path: String::new(),
            health_check_interval_secs: String::new(),
        }],
        port: port.map(|value| value.to_string()).unwrap_or_default(),
        protocol: "http1".to_string(),
        runtime: "process".to_string(),
        command: suggestion.command.clone(),
        workdir: suggestion.workdir.clone(),
        env_files: String::new(),
        depends_on: String::new(),
        autostart: false,
        health_path: String::new(),
        container_image: String::new(),
        container_args: String::new(),
        container_env: String::new(),
        container_volumes: String::new(),
        container_auto_remove: true,
    }
}

#[component]
pub(super) fn WizardServiceRuntimeFields(
    service_index: usize,
    entry: ServiceEntry,
    mut add_form: Signal<AddProjectInput>,
) -> Element {
    let selected_runtime = service_entry_runtime(&entry);
    let requires_command = !matches!(selected_runtime, ServiceRuntimeKind::Container);

    rsx! {
        label { class: "field",
            span { "Runtime" }
            select {
                value: "{service_runtime_value(selected_runtime)}",
                onchange: move |evt: Event<FormData>| {
                    let selected_runtime = parse_service_runtime(&evt.value());
                    let runtime = service_runtime_value(selected_runtime);
                    add_form.write().services[service_index].runtime = runtime.to_string();
                },
                option { value: "process", "Process" }
                option {
                    value: "container",
                    "Container"
                }
            }
        }
        if requires_command {
            label { class: "field field-wide",
                span { "Command" }
                input {
                    value: "{entry.command}",
                    placeholder: "pnpm dev",
                    oninput: move |evt: Event<FormData>| {
                        add_form.write().services[service_index].command =
                            normalize_service_command_input(&evt.value());
                    },
                }
            }
        } else {
            label { class: "field field-wide",
                span { "Container Image" }
                input {
                    value: "{entry.container_image}",
                    placeholder: "postgres:16",
                    oninput: move |evt: Event<FormData>| {
                        add_form.write().services[service_index].container_image = evt.value();
                    },
                }
            }
            label { class: "field field-wide",
                span { "Container Args" }
                input {
                    value: "{entry.container_args}",
                    placeholder: "-c shared_buffers=256MB, -c max_connections=200",
                    oninput: move |evt: Event<FormData>| {
                        add_form.write().services[service_index].container_args = evt.value();
                    },
                }
            }
            label { class: "field field-wide",
                span { "Container Env (comma/newline separated KEY=VALUE)" }
                textarea {
                    class: "field-input field-textarea",
                    value: "{entry.container_env}",
                    placeholder: "POSTGRES_DB=app
    POSTGRES_PASSWORD=secret",
                    oninput: move |evt: Event<FormData>| {
                        add_form.write().services[service_index].container_env = evt.value();
                    },
                }
            }
            label { class: "field field-wide",
                span { "Container Volumes (comma/newline separated)" }
                textarea {
                    class: "field-input field-textarea",
                    value: "{entry.container_volumes}",
                    placeholder: "/tmp/pg:/var/lib/postgresql/data",
                    oninput: move |evt: Event<FormData>| {
                        add_form.write().services[service_index].container_volumes = evt.value();
                    },
                }
            }
            button {
                class: if entry.container_auto_remove { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                onclick: move |_| {
                    let current = add_form().services[service_index].container_auto_remove;
                    add_form.write().services[service_index].container_auto_remove = !current;
                },
                if entry.container_auto_remove { "Container auto-remove: on" } else { "Container auto-remove: off" }
            }
        }
    }
}
