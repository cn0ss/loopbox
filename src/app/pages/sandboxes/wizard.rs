use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SandboxBlueprint {
    AutoDetect,
    Blank,
    Expo,
}

impl SandboxBlueprint {
    fn title(self) -> &'static str {
        match self {
            Self::AutoDetect => "Auto-detect",
            Self::Blank => "Blank",
            Self::Expo => "Expo App",
        }
    }

    fn kicker(self) -> &'static str {
        match self {
            Self::AutoDetect => "Recommended",
            Self::Blank => "Manual",
            Self::Expo => "Preset",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::AutoDetect => {
                "Scan package scripts, compose services, and known app types before you edit anything."
            }
            Self::Blank => {
                "Start from a clean sandbox and add each service, port, and dependency yourself."
            }
            Self::Expo => {
                "Seed a Metro/Expo dev service with an explicit port and an interactive terminal-friendly command."
            }
        }
    }
}

#[component]
pub(super) fn NewSandboxWizard(
    add_form_snapshot: AddProjectInput,
    service_host_previews: Vec<String>,
    mut add_form: Signal<AddProjectInput>,
    mut selected_project: Signal<Option<String>>,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    pending_auto_apply: Signal<Option<String>>,
    mut current_page: Signal<Page>,
) -> Element {
    let initial_browser_path = if add_form_snapshot.dir.trim().is_empty() {
        default_browser_path()
    } else {
        expand_tilde_path(&add_form_snapshot.dir)
    };
    let mut step = use_signal(|| 1_u8);
    let mut selected_blueprint_signal = use_signal(|| SandboxBlueprint::AutoDetect);
    let mut browser_path = use_signal(move || initial_browser_path.clone());
    let mut browser_refresh = use_signal(|| 0_u64);
    let mut discovery_suggestions = use_signal(Vec::<loopbox::DiscoverySuggestion>::new);
    let mut discovery_cache_key = use_signal(String::new);
    let mut detected_blueprint_signal = use_signal(|| None::<loopbox::ProjectBlueprintSuggestion>);
    let mut detected_blueprint_cache_key = use_signal(String::new);

    let directory_entries = use_memo(move || {
        let _refresh_tick = browser_refresh();
        list_child_directories(&browser_path())
    });

    use_effect(move || {
        let step_value = step();
        let dir = add_form().dir;
        let trimmed_dir = dir.trim().to_string();
        let cache_key = format!("{step_value}::{trimmed_dir}");

        if discovery_cache_key() == cache_key {
            return;
        }
        discovery_cache_key.set(cache_key);

        if step_value != 3 || trimmed_dir.is_empty() {
            discovery_suggestions.set(Vec::new());
            return;
        }

        let resolved_dir = expand_tilde_path(&trimmed_dir);
        discovery_suggestions
            .set(loopbox::discover_project_commands(&resolved_dir).unwrap_or_default());
    });

    use_effect(move || {
        let dir = add_form().dir;
        let trimmed_dir = dir.trim().to_string();
        if detected_blueprint_cache_key() == trimmed_dir {
            return;
        }
        detected_blueprint_cache_key.set(trimmed_dir.clone());

        if trimmed_dir.is_empty() {
            detected_blueprint_signal.set(None);
            return;
        }

        let resolved_dir = expand_tilde_path(&trimmed_dir);
        detected_blueprint_signal.set(
            loopbox::detect_project_blueprint(&resolved_dir)
                .ok()
                .flatten(),
        );
    });

    let current_step = step();
    let selected_blueprint = selected_blueprint_signal();
    let can_continue_project =
        !add_form_snapshot.name.trim().is_empty() && !add_form_snapshot.dir.trim().is_empty();
    let preview_services: Vec<ServiceEntry> = add_form_snapshot
        .services
        .iter()
        .filter(|entry| !entry.name.trim().is_empty())
        .cloned()
        .collect();
    let has_services = !preview_services.is_empty();
    let commands_ready = preview_services.iter().all(wizard_service_entry_is_ready);
    let can_continue_services = has_services && commands_ready;

    let discovery_suggestions = discovery_suggestions();
    let detected_blueprint = detected_blueprint_signal();
    let preflight_checks = if current_step == 4 {
        build_wizard_preflight(&add_form_snapshot, &config())
    } else {
        Vec::new()
    };
    let preflight_ok = preflight_checks.iter().all(|check| check.ok);

    let browser_path_value = browser_path();
    let browser_parent = parent_directory(&browser_path_value);

    rsx! {
        div { class: "page",
            div { class: "page-header",
                div { class: "page-header-left",
                    button {
                        class: "breadcrumb-link",
                        onclick: move |_| {
                            add_form.set(AddProjectInput::default());
                            selected_blueprint_signal.set(SandboxBlueprint::AutoDetect);
                            step.set(1);
                            current_page.set(Page::Sandboxes);
                        },
                        "\u{2190} Back to Sandboxes"
                    }
                }
            }

            h1 { class: "page-title wizard-title", "New Sandbox" }

            div { class: "wizard-steps",
                for (index, label) in [
                    (1_u8, "Blueprint"),
                    (2_u8, "Project"),
                    (3_u8, "Services"),
                    (4_u8, "Review"),
                ] {
                    {{
                        let unlocked = match index {
                            1 => true,
                            2 => true,
                            3 => can_continue_project,
                            4 => can_continue_project && can_continue_services,
                            _ => false,
                        };
                        let class_name = if current_step == index {
                            "wizard-step wizard-step-active"
                        } else if current_step > index {
                            "wizard-step wizard-step-done"
                        } else {
                            "wizard-step"
                        };

                        rsx! {
                            button {
                                class: "{class_name}",
                                disabled: !unlocked,
                                key: "{index}",
                                onclick: move |_| step.set(index),
                                span { class: "wizard-step-index", "{index}" }
                                span { class: "wizard-step-label", "{label}" }
                            }
                        }
                    }}
                }
            }

            if current_step == 1 {
                div { class: "wizard-pane",
                    p { class: "wizard-subtitle",
                        "Pick how guided this sandbox setup should be. You can still edit every generated field later."
                    }

                    div { class: "wizard-blueprint-grid",
                        for blueprint in [
                            SandboxBlueprint::AutoDetect,
                            SandboxBlueprint::Blank,
                            SandboxBlueprint::Expo,
                        ] {
                            {{
                                let is_selected = selected_blueprint == blueprint;
                                let card_class = if is_selected {
                                    "wizard-blueprint-card wizard-blueprint-card-active"
                                } else {
                                    "wizard-blueprint-card"
                                };
                                rsx! {
                                    button {
                                        key: "{blueprint.title()}",
                                        class: "{card_class}",
                                        onclick: move |_| selected_blueprint_signal.set(blueprint),
                                        div { class: "wizard-blueprint-head",
                                            span { class: "wizard-blueprint-kicker", "{blueprint.kicker()}" }
                                            if is_selected {
                                                span { class: "chip chip-success", "selected" }
                                            }
                                        }
                                        h2 { class: "wizard-blueprint-title", "{blueprint.title()}" }
                                        p { class: "wizard-blueprint-desc", "{blueprint.description()}" }
                                    }
                                }
                            }}
                        }
                    }

                    div { class: "wizard-footer wizard-footer-split",
                        div { class: "wizard-footer-copy",
                            p { class: "text-dim",
                                "Current mode: "
                                span { class: "wizard-inline-highlight", "{selected_blueprint.title()}" }
                            }
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| step.set(2),
                            "Continue to Project"
                        }
                    }
                }
            } else if current_step == 2 {
                div { class: "wizard-pane",
                    p { class: "wizard-subtitle",
                        "{wizard_project_step_subtitle(selected_blueprint)}"
                    }

                    div { class: "field-grid",
                        label { class: "field",
                            span { "Name" }
                            input {
                                value: "{add_form_snapshot.name}",
                                placeholder: "app1",
                                oninput: move |evt| add_form.write().name = evt.value(),
                            }
                        }
                        label { class: "field",
                            span { "Directory" }
                            input {
                                value: "{add_form_snapshot.dir}",
                                placeholder: "~/dev/app1",
                                oninput: move |evt| add_form.write().dir = evt.value(),
                            }
                        }
                    }

                    div { class: "wizard-inline-actions",
                        button {
                            class: "btn btn-sm btn-outline",
                            onclick: move |_| {
                                let current = add_form().dir;
                                let start_dir = if current.trim().is_empty() {
                                    Some(browser_path())
                                } else {
                                    Some(expand_tilde_path(&current))
                                };
                                match select_directory_via_native_dialog(start_dir.as_deref()) {
                                    Ok(Some(selected_dir)) => {
                                        let mut updated = add_form();
                                        apply_directory_to_form(&mut updated, &selected_dir);
                                        add_form.set(updated);
                                        browser_path.set(selected_dir.clone());
                                        notice.set(Some(Notice::success(format!(
                                            "Selected '{selected_dir}'."
                                        ))));
                                    }
                                    Ok(None) => {
                                        notice.set(Some(Notice::info(
                                            "Folder selection was cancelled."
                                        )));
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            },
                            "Browse..."
                        }
                        button {
                            class: "btn btn-sm btn-outline",
                            onclick: move |_| {
                                let typed = add_form().dir;
                                let target = if typed.trim().is_empty() {
                                    browser_path()
                                } else {
                                    expand_tilde_path(&typed)
                                };
                                browser_path.set(target);
                            },
                            "Open Path"
                        }
                        button {
                            class: "btn btn-sm btn-outline",
                            onclick: move |_| {
                                let typed = add_form().dir;
                                let target = if typed.trim().is_empty() {
                                    browser_path()
                                } else {
                                    typed
                                };
                                match ensure_directory_exists(&target) {
                                    Ok(valid) => {
                                        browser_path.set(valid.clone());
                                        let mut updated = add_form();
                                        apply_directory_to_form(&mut updated, &valid);
                                        add_form.set(updated);
                                        notice.set(Some(Notice::info(format!(
                                            "Selected '{valid}'."
                                        ))));
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            },
                            "Use Path"
                        }
                    }

                    div { class: "dir-browser",
                        div { class: "dir-browser-header",
                            span { class: "dir-browser-path", "{browser_path_value}" }
                            div { class: "dir-browser-actions",
                                if let Some(parent) = browser_parent {
                                    button {
                                        class: "btn btn-sm btn-outline",
                                        onclick: move |_| browser_path.set(parent.clone()),
                                        "Up"
                                    }
                                }
                                button {
                                    class: "btn btn-sm btn-outline",
                                    onclick: move |_| {
                                        browser_refresh.with_mut(|tick| *tick = tick.wrapping_add(1));
                                    },
                                    "Refresh"
                                }
                                button {
                                    class: "btn btn-sm btn-outline",
                                    onclick: move |_| {
                                        let current = browser_path();
                                        let mut updated = add_form();
                                        apply_directory_to_form(&mut updated, &current);
                                        add_form.set(updated);
                                        notice.set(Some(Notice::info(format!(
                                            "Selected '{current}'."
                                        ))));
                                    },
                                    "Select Current"
                                }
                            }
                        }
                        div { class: "dir-browser-list",
                            match directory_entries() {
                                Ok(entries) => {
                                    if entries.is_empty() {
                                        rsx! { p { class: "text-dim", "No subdirectories found." } }
                                    } else {
                                        rsx! {
                                            for entry in entries {
                                                {{
                                                    let open_target = entry.path.clone();
                                                    let select_target = entry.path.clone();
                                                    let display_name = entry.name.clone();
                                                    rsx! {
                                                        div { class: "dir-browser-row", key: "{entry.path}",
                                                            button {
                                                                class: "dir-browser-open",
                                                                onclick: move |_| browser_path.set(open_target.clone()),
                                                                "{display_name}"
                                                            }
                                                            button {
                                                                class: "btn btn-sm btn-outline",
                                                                onclick: move |_| {
                                                                    let mut updated = add_form();
                                                                    apply_directory_to_form(&mut updated, &select_target);
                                                                    add_form.set(updated);
                                                                    browser_path.set(select_target.clone());
                                                                    notice.set(Some(Notice::info(format!(
                                                                        "Selected '{select_target}'."
                                                                    ))));
                                                                },
                                                                "Select"
                                                            }
                                                        }
                                                    }
                                                }}
                                            }
                                        }
                                    }
                                }
                                Err(err) => rsx! { p { class: "text-dim", "{err}" } },
                            }
                        }
                    }

                    if let Some(detected) = &detected_blueprint {
                        div { class: "wizard-detected-card",
                            div { class: "wizard-detected-head",
                                span { class: "wizard-detected-title", "Detected project type" }
                                span { class: "chip chip-accent", "{project_blueprint_label(detected)}" }
                            }
                            p { class: "wizard-detected-line",
                                "Workdir: "
                                span { class: "wizard-review-mono", "{detected.workdir}" }
                            }
                            p { class: "wizard-detected-line",
                                "Reason: {detected.reason}"
                            }
                            if selected_blueprint != SandboxBlueprint::Expo
                                && matches!(detected.kind, loopbox::ProjectBlueprintKind::Expo)
                            {
                                button {
                                    class: "btn btn-sm btn-outline",
                                    onclick: move |_| selected_blueprint_signal.set(SandboxBlueprint::Expo),
                                    "Switch to Expo Template"
                                }
                            }
                        }
                    } else if selected_blueprint == SandboxBlueprint::AutoDetect
                        && !add_form_snapshot.dir.trim().is_empty()
                    {
                        div { class: "wizard-detected-card wizard-detected-card-muted",
                            div { class: "wizard-detected-head",
                                span { class: "wizard-detected-title", "Detection preview" }
                            }
                            p { class: "wizard-detected-line",
                                "No specialized blueprint detected yet. Auto-detect will fall back to scripts and compose services."
                            }
                        }
                    }

                    div { class: "wizard-footer wizard-footer-split",
                        button {
                            class: "btn btn-outline",
                            onclick: move |_| step.set(1),
                            "Back"
                        }
                        button {
                            class: "btn btn-primary",
                            disabled: !can_continue_project,
                            onclick: move |_| {
                                let mut updated = add_form();
                                let picked_dir = if updated.dir.trim().is_empty() {
                                    browser_path()
                                } else {
                                    updated.dir.clone()
                                };
                                match ensure_directory_exists(&picked_dir) {
                                    Ok(valid) => {
                                        apply_directory_to_form(&mut updated, &valid);
                                        match build_services_for_blueprint(
                                            &valid,
                                            selected_blueprint_signal(),
                                            detected_blueprint_signal().as_ref(),
                                            &config(),
                                        ) {
                                            Ok(services) => {
                                                let count = services.iter()
                                                    .filter(|service| !service.name.trim().is_empty())
                                                    .count();
                                                updated.services = services;
                                                add_form.set(updated);
                                                browser_path.set(valid.clone());
                                                step.set(3);
                                                notice.set(Some(Notice::success(format!(
                                                    "{} from '{}'.",
                                                    blueprint_apply_message(selected_blueprint_signal(), count),
                                                    valid,
                                                ))));
                                            }
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            },
                            "{wizard_project_primary_action_label(selected_blueprint)}"
                        }
                    }
                }
            } else if current_step == 3 {
                div { class: "wizard-pane",
                    p { class: "wizard-subtitle",
                        "{wizard_service_step_subtitle()}"
                    }

                    div { class: "wizard-service-mode-banner",
                        span { class: "chip chip-accent", "{selected_blueprint.title()}" }
                        p { class: "text-dim",
                            "{wizard_service_mode_hint(selected_blueprint, detected_blueprint.as_ref())}"
                        }
                    }

                    div { class: "service-section",
                        div { class: "service-section-header",
                            span { "Services" }
                            div { class: "service-section-actions",
                                button {
                                    class: "btn btn-sm btn-outline",
                                    onclick: move |_| {
                                        let snapshot = add_form();
                                        if snapshot.dir.trim().is_empty() {
                                            notice.set(Some(Notice::error(
                                                "Set a project directory first."
                                            )));
                                            return;
                                        }
                                        let project_dir = expand_tilde_path(&snapshot.dir);
                                        match loopbox::discover_project_commands(&project_dir) {
                                            Ok(suggestions) => {
                                                let mut updated = snapshot.clone();
                                                let matched = align_commands_with_discovery(
                                                    &mut updated.services,
                                                    &suggestions,
                                                );
                                                add_form.set(updated);
                                                notice.set(Some(Notice::info(format!(
                                                    "Matched commands for {matched} service(s)."
                                                ))));
                                            }
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    },
                                    "Match Commands"
                                }
                                if selected_blueprint != SandboxBlueprint::Blank {
                                    button {
                                        class: "btn btn-sm btn-outline",
                                        onclick: move |_| {
                                            let snapshot = add_form();
                                            match build_services_for_blueprint(
                                                &snapshot.dir,
                                                selected_blueprint_signal(),
                                                detected_blueprint_signal().as_ref(),
                                                &config(),
                                            ) {
                                                Ok(services) => {
                                                    let count = services.len();
                                                    let mut updated = snapshot.clone();
                                                    updated.services = services;
                                                    add_form.set(updated);
                                                    notice.set(Some(Notice::success(format!(
                                                        "{} ({count} service(s)).",
                                                        wizard_service_regenerate_notice(selected_blueprint_signal())
                                                    ))));
                                                }
                                                Err(err) => notice.set(Some(Notice::error(err))),
                                            }
                                        },
                                        "{wizard_service_regenerate_label(selected_blueprint_signal())}"
                                    }
                                }
                                button {
                                    class: "btn btn-sm btn-outline",
                                    onclick: move |_| {
                                        add_form.write().services.push(blank_service_entry());
                                    },
                                    "+ Service"
                                }
                            }
                        }
                        div { class: "service-edit-list",
                            for (i, entry) in add_form_snapshot.services.iter().enumerate() {
                                div { class: "service-edit-card", key: "{i}",
                                    div { class: "service-edit-head",
                                        span { class: "service-edit-num", "#{i + 1}" }
                                        if !entry.name.is_empty() {
                                            span { class: "service-edit-name", "{entry.name}" }
                                        }
                                        button {
                                            class: "service-edit-remove",
                                            onclick: move |_| {
                                                add_form.write().services.remove(i);
                                            },
                                            "\u{00D7}"
                                        }
                                    }
                                    div { class: "service-edit-grid",
                                        label { class: "field",
                                            span { "Name" }
                                            input {
                                                value: "{entry.name}",
                                                placeholder: "service",
                                                oninput: move |evt: Event<FormData>| {
                                                    add_form.write().services[i].name = evt.value();
                                                },
                                            }
                                        }
                                        div { class: "field field-wide",
                                            span { "Ports (optional, multiple supported)" }
                                            div { class: "service-port-list",
                                                for (port_idx, port_entry) in service_entry_port_rows(entry).iter().enumerate() {
                                                    div { class: "service-port-row", key: "new-port-{i}-{port_idx}",
                                                        input {
                                                            value: "{port_entry.port}",
                                                            placeholder: "Port (e.g. 8080)",
                                                            oninput: move |evt: Event<FormData>| {
                                                                add_form.with_mut(|form| {
                                                                    if let Some(service) = form.services.get_mut(i) {
                                                                        if service.ports.is_empty() {
                                                                            service.ports = service_entry_port_rows(service);
                                                                        }
                                                                        if let Some(port) = service.ports.get_mut(port_idx) {
                                                                            port.port = evt.value();
                                                                        }
                                                                        sync_service_entry_primary_port(service);
                                                                    }
                                                                });
                                                            },
                                                        }
                                                        select {
                                                            value: "{port_entry.protocol}",
                                                            onchange: move |evt: Event<FormData>| {
                                                                let raw = evt.value();
                                                                let canonical = parse_service_protocol(&raw)
                                                                    .map(|protocol| service_protocol_value(&protocol).to_string())
                                                                    .unwrap_or_else(|| "http1".to_string());
                                                                add_form.with_mut(|form| {
                                                                    if let Some(service) = form.services.get_mut(i) {
                                                                        if service.ports.is_empty() {
                                                                            service.ports = service_entry_port_rows(service);
                                                                        }
                                                                        if let Some(port) = service.ports.get_mut(port_idx) {
                                                                            port.protocol = canonical.clone();
                                                                        }
                                                                        sync_service_entry_primary_port(service);
                                                                    }
                                                                });
                                                            },
                                                            option { value: "http1", "http1" }
                                                            option { value: "grpc_h2c", "grpc_h2c" }
                                                            option { value: "tcp_passthrough", "tcp_passthrough" }
                                                        }
                                                        input {
                                                            value: "{port_entry.health_path}",
                                                            placeholder: "Health target (HTTP path or gRPC service, optional)",
                                                            oninput: move |evt: Event<FormData>| {
                                                                add_form.with_mut(|form| {
                                                                    if let Some(service) = form.services.get_mut(i) {
                                                                        if service.ports.is_empty() {
                                                                            service.ports = service_entry_port_rows(service);
                                                                        }
                                                                        if let Some(port) = service.ports.get_mut(port_idx) {
                                                                            port.health_path = evt.value();
                                                                        }
                                                                        sync_service_entry_primary_port(service);
                                                                    }
                                                                });
                                                            },
                                                        }
                                                        button {
                                                            class: "btn btn-sm btn-outline",
                                                            onclick: move |_| {
                                                                add_form.with_mut(|form| {
                                                                    if let Some(service) = form.services.get_mut(i) {
                                                                        if service.ports.is_empty() {
                                                                            service.ports = service_entry_port_rows(service);
                                                                        }
                                                                        if service.ports.len() > 1 && port_idx < service.ports.len() {
                                                                            service.ports.remove(port_idx);
                                                                        } else if let Some(first) = service.ports.first_mut() {
                                                                            *first = blank_service_port_entry();
                                                                        }
                                                                        sync_service_entry_primary_port(service);
                                                                    }
                                                                });
                                                            },
                                                            "Remove"
                                                        }
                                                    }
                                                }
                                            }
                                            button {
                                                class: "btn btn-sm btn-outline",
                                                onclick: move |_| {
                                                    add_form.with_mut(|form| {
                                                        if let Some(service) = form.services.get_mut(i) {
                                                            service.ports.push(blank_service_port_entry());
                                                            sync_service_entry_primary_port(service);
                                                        }
                                                    });
                                                },
                                                "+ Port"
                                            }
                                        }
                                        WizardServiceRuntimeFields {
                                            service_index: i,
                                            entry: entry.clone(),
                                            add_form,
                                        }
                                        label { class: "field",
                                            span { "Workdir" }
                                            input {
                                                value: "{entry.workdir}",
                                                placeholder: "optional",
                                                oninput: move |evt: Event<FormData>| {
                                                    add_form.write().services[i].workdir = evt.value();
                                                },
                                            }
                                        }
                                        label { class: "field",
                                            span { "Env Files" }
                                            input {
                                                value: "{entry.env_files}",
                                                placeholder: ".env,.env.local",
                                                oninput: move |evt: Event<FormData>| {
                                                    add_form.write().services[i].env_files = evt.value();
                                                },
                                            }
                                        }
                                        label { class: "field",
                                            span { "Depends On" }
                                            input {
                                                value: "{entry.depends_on}",
                                                placeholder: "gateway,db",
                                                oninput: move |evt: Event<FormData>| {
                                                    add_form.write().services[i].depends_on = evt.value();
                                                },
                                            }
                                        }
                                    }
                                    {{
                                        let suggestion = if entry.name.trim().is_empty() {
                                            None
                                        } else {
                                            loopbox::best_command_for_service(
                                                &entry.name,
                                                &discovery_suggestions,
                                            )
                                        };
                                        if let Some(suggestion) = suggestion {
                                            let reason = discovery_reason(&entry.name, &suggestion);
                                            let package = suggestion
                                                .package_name
                                                .clone()
                                                .unwrap_or_else(|| "(no package name)".to_string());
                                            rsx! {
                                                div { class: "service-discovery",
                                                    div { class: "service-discovery-chips",
                                                        span { class: "chip chip-accent", "score {suggestion.confidence}" }
                                                        span { class: "chip", "{suggestion.origin}" }
                                                    }
                                                    p { class: "service-discovery-line",
                                                        "Suggested from {package} via script '{suggestion.script_name}'."
                                                    }
                                                    p { class: "service-discovery-line service-discovery-dim", "{reason}" }
                                                }
                                            }
                                        } else if !entry.name.trim().is_empty() {
                                            rsx! {
                                                p { class: "service-discovery-line service-discovery-dim",
                                                    "No package/script match found for this service name yet."
                                                }
                                            }
                                        } else {
                                            rsx! {}
                                        }
                                    }}
                                    div { class: "service-edit-foot",
                                        button {
                                            class: if entry.autostart { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                                            onclick: move |_| {
                                                let current = add_form().services[i].autostart;
                                                add_form.write().services[i].autostart = !current;
                                            },
                                            if entry.autostart { "Autostart: on" } else { "Autostart: off" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !has_services {
                        p { class: "text-dim", "Add at least one named service before you continue." }
                    } else if !commands_ready {
                        p { class: "text-dim", "{wizard_service_requirement_hint()}" }
                    }

                    div { class: "wizard-footer wizard-footer-split",
                        button {
                            class: "btn btn-outline",
                            onclick: move |_| step.set(1),
                            "Back"
                        }
                        button {
                            class: "btn btn-primary",
                            disabled: !can_continue_services,
                            onclick: move |_| step.set(4),
                            "Continue to Review"
                        }
                    }
                }
            } else {
                div { class: "wizard-pane",
                    p { class: "wizard-subtitle",
                        "Final check: verify hosts, ports, and commands, then create the sandbox."
                    }

                    div { class: "wizard-review-grid",
                        div { class: "wizard-review-card",
                            span { class: "wizard-review-label", "Project" }
                            p { class: "wizard-review-value", "{add_form_snapshot.name}" }
                        }
                        div { class: "wizard-review-card",
                            span { class: "wizard-review-label", "Directory" }
                            p { class: "wizard-review-mono", "{add_form_snapshot.dir}" }
                        }
                        div { class: "wizard-review-card",
                            span { class: "wizard-review-label", "IP" }
                            p { class: "wizard-review-mono",
                                if add_form_snapshot.ip.trim().is_empty() {
                                    "auto"
                                } else {
                                    "{add_form_snapshot.ip}"
                                }
                            }
                        }
                        div { class: "wizard-review-card",
                            span { class: "wizard-review-label", "Services" }
                            p { class: "wizard-review-value", "{preview_services.len()}" }
                        }
                    }

                    div { class: "field field-generated",
                        span { "Generated Hosts" }
                        if service_host_previews.is_empty() {
                            p { class: "field-generated-line field-generated-dim",
                                "Add named services to generate hostnames."
                            }
                        } else {
                            for host in &service_host_previews {
                                p { class: "field-generated-line", key: "{host}", "{host}" }
                            }
                        }
                    }

                    div { class: "wizard-preflight",
                        h3 { "Preflight Checks" }
                        if preflight_checks.is_empty() {
                            p { class: "text-dim", "No checks available yet." }
                        } else {
                            for (idx, check) in preflight_checks.iter().enumerate() {
                                div {
                                    class: if check.ok {
                                        "wizard-check wizard-check-ok"
                                    } else {
                                        "wizard-check wizard-check-bad"
                                    },
                                    key: "{idx}",
                                    span { class: "wizard-check-icon",
                                        if check.ok { "\u{2713}" } else { "\u{26A0}" }
                                    }
                                    div { class: "wizard-check-body",
                                        p { class: "wizard-check-title", "{check.title}" }
                                        p { class: "wizard-check-detail", "{check.detail}" }
                                    }
                                }
                            }
                        }
                    }

                    div { class: "service-edit-list wizard-review-services",
                        for service in &preview_services {
                            div { class: "service-edit-card", key: "{service.name}",
                                div { class: "service-edit-head",
                                    span { class: "service-edit-name", "{service.name}" }
                                    span { class: "service-edit-num",
                                        {
                                            let port_labels = service_entry_port_rows(service)
                                                .into_iter()
                                                .filter_map(|port_entry| {
                                                    let port = port_entry.port.trim().to_string();
                                                    if port.is_empty() {
                                                        return None;
                                                    }
                                                    let protocol = parse_service_protocol(&port_entry.protocol)
                                                        .map(|value| service_protocol_value(&value).to_string())
                                                        .unwrap_or_else(|| "http1".to_string());
                                                    Some(format!(":{port}/{protocol}"))
                                                })
                                                .collect::<Vec<_>>();
                                            if port_labels.is_empty() {
                                                "no port".to_string()
                                            } else {
                                                port_labels.join(", ")
                                            }
                                        }
                                    }
                                }
                                div { class: "service-edit-grid",
                                    div { class: "field field-wide",
                                        span { "Command" }
                                        p { class: "wizard-review-mono", "{service.command}" }
                                    }
                                    div { class: "field field-wide",
                                        span { "Workdir" }
                                        p { class: "wizard-review-mono", "{service.workdir}" }
                                    }
                                    if !service.depends_on.trim().is_empty() {
                                        div { class: "field field-wide",
                                            span { "Depends On" }
                                            p { class: "wizard-review-mono", "{service.depends_on}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !preflight_ok {
                        p { class: "text-dim",
                            "Resolve failed preflight checks before creating this sandbox."
                        }
                    }

                    div { class: "wizard-footer wizard-footer-split",
                        button {
                            class: "btn btn-outline",
                            onclick: move |_| step.set(3),
                            "Back"
                        }
                        button {
                            class: "btn btn-primary",
                            disabled: !preflight_ok,
                            onclick: move |_| {
                                let input = add_form();
                                let previous = config();
                                let add_result = {
                                    let mut cfg = config.write();
                                    loopbox::add_project(&mut cfg, &input)
                                };

                                match add_result {
                                    Ok(name) => {
                                        selected_project.set(Some(name.clone()));
                                        add_form.set(AddProjectInput::default());
                                        selected_blueprint_signal.set(SandboxBlueprint::AutoDetect);
                                        step.set(1);
                                        browser_path.set(default_browser_path());
                                        current_page.set(Page::Sandboxes);
                                        persist_config_and_apply(
                                            config,
                                            notice,
                                            pending_auto_apply,
                                            format!("Added '{name}'."),
                                            Some(previous),
                                        );
                                    }
                                    Err(err) => notice.set(Some(Notice::error(err))),
                                }
                            },
                            "Add Sandbox"
                        }
                    }
                }
            }
        }
    }
}

fn wizard_project_step_subtitle(blueprint: SandboxBlueprint) -> &'static str {
    match blueprint {
        SandboxBlueprint::AutoDetect => {
            "Choose your project directory. Loopbox will scan scripts, compose services, and known app types before generating services."
        }
        SandboxBlueprint::Blank => {
            "Choose your project directory. Loopbox will carry over only the sandbox identity and start you with a blank service."
        }
        SandboxBlueprint::Expo => {
            "Choose your Expo app directory. Loopbox will seed an Expo/Metro service with an explicit port and interactive CLI command."
        }
    }
}

fn wizard_project_primary_action_label(blueprint: SandboxBlueprint) -> &'static str {
    match blueprint {
        SandboxBlueprint::AutoDetect => "Analyze & Continue",
        SandboxBlueprint::Blank => "Start Blank Sandbox",
        SandboxBlueprint::Expo => "Use Expo Template",
    }
}

fn wizard_service_regenerate_label(blueprint: SandboxBlueprint) -> &'static str {
    match blueprint {
        SandboxBlueprint::AutoDetect => "Regenerate from Detection",
        SandboxBlueprint::Blank => "Regenerate",
        SandboxBlueprint::Expo => "Reapply Expo Template",
    }
}

fn wizard_service_regenerate_notice(blueprint: SandboxBlueprint) -> &'static str {
    match blueprint {
        SandboxBlueprint::AutoDetect => "Refreshed services from detection",
        SandboxBlueprint::Blank => "Regenerated services",
        SandboxBlueprint::Expo => "Reapplied Expo template",
    }
}

fn wizard_service_mode_hint(
    blueprint: SandboxBlueprint,
    detected: Option<&loopbox::ProjectBlueprintSuggestion>,
) -> &'static str {
    match (blueprint, detected.map(|value| value.kind)) {
        (SandboxBlueprint::AutoDetect, Some(loopbox::ProjectBlueprintKind::Expo)) => {
            "Auto-detect found an Expo app. You can keep the generated mobile service or switch fully to the Expo preset."
        }
        (SandboxBlueprint::AutoDetect, None) => {
            "Loopbox is using script and compose discovery here. Fine-tune names, ports, and workdirs before review."
        }
        (SandboxBlueprint::Blank, _) => {
            "This mode stays intentionally minimal. Add only the services you actually need."
        }
        (SandboxBlueprint::Expo, _) => {
            "This preset is optimized for interactive Expo CLI workflows and explicit Metro port handling."
        }
    }
}

fn project_blueprint_label(suggestion: &loopbox::ProjectBlueprintSuggestion) -> &'static str {
    match suggestion.kind {
        loopbox::ProjectBlueprintKind::Expo => "Expo app",
    }
}

fn blueprint_apply_message(blueprint: SandboxBlueprint, service_count: usize) -> String {
    match blueprint {
        SandboxBlueprint::AutoDetect => format!("Prepared {service_count} detected service(s)"),
        SandboxBlueprint::Blank => "Started a blank sandbox setup".to_string(),
        SandboxBlueprint::Expo => "Prepared the Expo service template".to_string(),
    }
}

fn build_services_for_blueprint(
    project_dir: &str,
    blueprint: SandboxBlueprint,
    detected: Option<&loopbox::ProjectBlueprintSuggestion>,
    config: &LoopboxConfig,
) -> Result<Vec<ServiceEntry>, String> {
    match blueprint {
        SandboxBlueprint::Blank => Ok(vec![blank_service_entry()]),
        SandboxBlueprint::AutoDetect => {
            if matches!(
                detected.map(|value| value.kind),
                Some(loopbox::ProjectBlueprintKind::Expo)
            ) {
                build_expo_template_services(project_dir, detected, config)
            } else {
                build_discovered_services(project_dir)
            }
        }
        SandboxBlueprint::Expo => build_expo_template_services(project_dir, detected, config),
    }
}

fn build_expo_template_services(
    project_dir: &str,
    detected: Option<&loopbox::ProjectBlueprintSuggestion>,
    config: &LoopboxConfig,
) -> Result<Vec<ServiceEntry>, String> {
    let resolved_dir = ensure_directory_exists(project_dir)?;
    let detected = match detected {
        Some(detected) if matches!(detected.kind, loopbox::ProjectBlueprintKind::Expo) => {
            Some(detected.clone())
        }
        _ => loopbox::detect_project_blueprint(&resolved_dir)?,
    };

    let (workdir, command, package_name) = if let Some(detected) = detected {
        (
            detected.workdir,
            detected.command,
            detected.package_name.unwrap_or_default(),
        )
    } else {
        (
            resolved_dir.clone(),
            "npx expo start".to_string(),
            String::new(),
        )
    };
    let metro_port = suggest_expo_port(config);

    let mut entry = wizard_blank_service_entry_base();
    entry.name = suggested_expo_service_name(&workdir, &package_name);
    entry.runtime = "process".to_string();
    entry.command = command;
    entry.workdir = workdir;
    entry.ports = vec![ServicePortEntry {
        port: metro_port.to_string(),
        protocol: "http1".to_string(),
        health_path: String::new(),
    }];
    entry.autostart = false;
    sync_service_entry_primary_port(&mut entry);

    Ok(vec![entry])
}

fn suggest_expo_port(config: &LoopboxConfig) -> u16 {
    let mut reserved = BTreeSet::new();
    for project in config.projects.values() {
        for service in &project.services {
            for port in loopbox::service_ports(service) {
                reserved.insert(port.port);
            }
        }
    }

    for candidate in 8081_u16..=8999_u16 {
        if reserved.contains(&candidate) {
            continue;
        }
        if is_port_reachable("127.0.0.1", candidate, 60) {
            continue;
        }
        return candidate;
    }

    8081
}

fn suggested_expo_service_name(workdir: &str, package_name: &str) -> String {
    let from_package = sanitize_identifier(package_name);
    if !from_package.is_empty() && from_package != "app" {
        return from_package;
    }

    let from_dir = Path::new(workdir)
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_identifier)
        .unwrap_or_default();
    if !from_dir.is_empty() && from_dir != "app" {
        return from_dir;
    }

    "mobile".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WizardPreflightCheck {
    title: String,
    detail: String,
    ok: bool,
}

pub(super) fn build_wizard_preflight(
    form: &AddProjectInput,
    config: &LoopboxConfig,
) -> Vec<WizardPreflightCheck> {
    let mut checks = Vec::new();

    let project_dir = ensure_directory_exists(&form.dir);
    match &project_dir {
        Ok(path) => checks.push(WizardPreflightCheck {
            title: "Project directory".to_string(),
            detail: format!("Using '{path}'."),
            ok: true,
        }),
        Err(err) => checks.push(WizardPreflightCheck {
            title: "Project directory".to_string(),
            detail: err.clone(),
            ok: false,
        }),
    }

    let mut candidate = config.clone();
    let mut predicted_ip = None::<String>;
    let config_result = loopbox::add_project(&mut candidate, form);
    match config_result {
        Ok(project_name) => {
            predicted_ip = candidate
                .projects
                .get(&project_name)
                .map(|project| project.ip.clone());
            checks.push(WizardPreflightCheck {
                title: "Configuration validity".to_string(),
                detail: "Project name, IP, services, and hostnames are valid.".to_string(),
                ok: true,
            })
        }
        Err(err) => checks.push(WizardPreflightCheck {
            title: "Configuration validity".to_string(),
            detail: err,
            ok: false,
        }),
    }

    let mut invalid_workdirs = Vec::new();
    for service in form
        .services
        .iter()
        .filter(|service| !service.name.trim().is_empty())
    {
        let candidate_dir = if service.workdir.trim().is_empty() {
            project_dir
                .as_ref()
                .cloned()
                .unwrap_or_else(|_| expand_tilde_path(&form.dir))
        } else {
            expand_tilde_path(&service.workdir)
        };

        let path = Path::new(&candidate_dir);
        if !path.exists() || !path.is_dir() {
            invalid_workdirs.push(format!("{} -> {}", service.name.trim(), candidate_dir));
        }
    }
    if invalid_workdirs.is_empty() {
        checks.push(WizardPreflightCheck {
            title: "Service workdirs".to_string(),
            detail: "All service workdirs exist.".to_string(),
            ok: true,
        });
    } else {
        checks.push(WizardPreflightCheck {
            title: "Service workdirs".to_string(),
            detail: format!("Missing or invalid: {}", invalid_workdirs.join(", ")),
            ok: false,
        });
    }

    let ip = predicted_ip.unwrap_or_else(|| form.ip.trim().to_string());
    if ip.trim().is_empty() {
        checks.push(WizardPreflightCheck {
            title: "Port availability".to_string(),
            detail: "Skipped (could not determine target IP yet).".to_string(),
            ok: true,
        });
    } else {
        let mut busy_ports = Vec::new();
        for service in form
            .services
            .iter()
            .filter(|service| !service.name.trim().is_empty())
        {
            for port in service_entry_configured_ports(service) {
                if is_port_reachable(&ip, port, 120) {
                    busy_ports.push(format!("{ip}:{port} ({})", service.name.trim()));
                }
            }
        }

        if busy_ports.is_empty() {
            checks.push(WizardPreflightCheck {
                title: "Port availability".to_string(),
                detail: format!("No occupied configured ports on {ip}."),
                ok: true,
            });
        } else {
            checks.push(WizardPreflightCheck {
                title: "Port availability".to_string(),
                detail: format!("Already in use: {}", busy_ports.join(", ")),
                ok: false,
            });
        }
    }

    checks
}

pub(super) fn is_port_reachable(ip: &str, port: u16, timeout_ms: u64) -> bool {
    let Ok(mut addrs) = (ip, port).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok()
}

pub(super) fn apply_directory_to_form(form: &mut AddProjectInput, directory: &str) {
    let expanded = expand_tilde_path(directory);
    if expanded.is_empty() {
        return;
    }

    form.dir = expanded.clone();
    if form.name.trim().is_empty() {
        if let Some(inferred_name) = infer_project_name_from_directory(&expanded) {
            form.name = inferred_name;
        }
    }
}

pub(super) fn infer_project_name_from_directory(directory: &str) -> Option<String> {
    Path::new(directory)
        .file_name()
        .and_then(|name| name.to_str())
        .map(preview_project_name)
}

pub(super) fn blank_service_entry() -> ServiceEntry {
    let mut entry = wizard_blank_service_entry_base();
    sync_service_entry_primary_port(&mut entry);
    entry
}

pub(super) fn align_commands_with_discovery(
    entries: &mut [ServiceEntry],
    suggestions: &[loopbox::DiscoverySuggestion],
) -> usize {
    let mut matched = 0_usize;
    for service in entries {
        if service.name.trim().is_empty() {
            continue;
        }
        if let Some(suggestion) = loopbox::best_command_for_service(&service.name, suggestions) {
            service.command = suggestion.command;
            if service.workdir.trim().is_empty() {
                service.workdir = suggestion.workdir;
            }
            matched += 1;
        }
    }
    matched
}

pub(super) fn discovery_reason(
    service_name: &str,
    suggestion: &loopbox::DiscoverySuggestion,
) -> String {
    let needle = service_name.trim().to_lowercase();
    if needle.is_empty() {
        return "No service name provided yet.".to_string();
    }

    let mut reasons = Vec::new();
    if suggestion
        .package_name
        .as_ref()
        .is_some_and(|name| name.to_lowercase().contains(&needle))
    {
        reasons.push("package name match");
    }
    if suggestion.script_name.to_lowercase().contains(&needle) {
        reasons.push("script name match");
    }
    if suggestion.workdir.to_lowercase().contains(&needle) {
        reasons.push("workdir match");
    }
    if reasons.is_empty() {
        reasons.push("script priority fallback");
    }
    format!("Reason: {}.", reasons.join(", "))
}

pub(super) fn build_discovered_services(project_dir: &str) -> Result<Vec<ServiceEntry>, String> {
    let resolved_dir = ensure_directory_exists(project_dir)?;
    let mut compose_error = None::<String>;

    match loopbox::discover_compose_services(&resolved_dir) {
        Ok(Some(compose)) => {
            if !compose.services.is_empty() {
                let services = build_compose_discovered_services(&resolved_dir, &compose.services);
                if !services.is_empty() {
                    return Ok(services);
                }
                compose_error = Some(format!(
                    "Compose file '{}' did not produce importable services.",
                    compose.compose_file
                ));
            }
        }
        Ok(None) => {}
        Err(err) => compose_error = Some(err),
    }

    let suggestions = loopbox::discover_project_commands(&resolved_dir)?;
    if suggestions.is_empty() {
        if let Some(compose_error) = compose_error {
            return Err(format!(
                "{compose_error} No package.json scripts found in this directory."
            ));
        }
        return Err("No package.json scripts found in this directory.".to_string());
    }

    let mut selected = Vec::new();
    let mut seen_workdirs = HashSet::new();
    for suggestion in suggestions {
        if seen_workdirs.insert(suggestion.workdir.clone()) {
            selected.push(suggestion);
        }
    }

    if selected.is_empty() {
        return Err("No runnable scripts found for this project directory.".to_string());
    }

    let mut used_names = HashSet::new();
    let mut used_ports = BTreeSet::new();
    let mut services = Vec::new();
    for (index, suggestion) in selected.into_iter().enumerate() {
        let base_name = service_name_from_suggestion(&suggestion);
        let name = unique_service_name(base_name, &mut used_names);
        let port = suggest_service_port(&name, index, &mut used_ports);
        let mut service = wizard_discovered_service_entry(name, port, &suggestion);
        sync_service_entry_primary_port(&mut service);
        services.push(service);
    }

    Ok(services)
}

fn build_compose_discovered_services(
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

pub(super) fn service_name_from_suggestion(suggestion: &loopbox::DiscoverySuggestion) -> String {
    let mut candidate = suggestion
        .package_name
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();

    if !candidate.is_empty() {
        if let Some(last) = candidate.rsplit('/').next() {
            candidate = last.to_string();
        }
        candidate = candidate.trim_start_matches('@').to_string();
    }

    if candidate.is_empty() {
        candidate = Path::new(&suggestion.workdir)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("service")
            .to_string();
    }

    let normalized = sanitize_identifier(&candidate);
    if normalized.is_empty() {
        "service".to_string()
    } else {
        normalized
    }
}

pub(super) fn sanitize_identifier(raw: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_separator = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }

        if matches!(ch, '-' | '_' | '.' | ' ') && !normalized.is_empty() && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }
    normalized.trim_matches('-').trim_matches('_').to_string()
}

pub(super) fn unique_service_name(base_name: String, used: &mut HashSet<String>) -> String {
    let root = if base_name.trim().is_empty() {
        "service".to_string()
    } else {
        base_name
    };
    if used.insert(root.clone()) {
        return root;
    }

    let mut index = 2_u16;
    loop {
        let candidate = format!("{root}-{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index = index.saturating_add(1);
    }
}

pub(super) fn suggest_service_port(
    name: &str,
    index: usize,
    used: &mut BTreeSet<u16>,
) -> Option<u16> {
    let lowered = name.to_lowercase();
    if lowered.contains("worker")
        || lowered.contains("queue")
        || lowered.contains("job")
        || lowered.contains("cli")
        || lowered.contains("convex")
    {
        return None;
    }

    let mut candidates = Vec::new();
    if lowered.contains("front") || lowered.contains("web") || lowered.contains("ui") {
        candidates.push(5173);
    } else if lowered.contains("back") || lowered.contains("api") || lowered.contains("server") {
        candidates.push(8080);
    } else if lowered.contains("admin") {
        candidates.push(3000);
    } else if lowered.contains("worker") || lowered.contains("queue") {
        candidates.push(9000);
    }

    candidates.extend([3000, 4000, 5000, 5173, 6006, 7000, 8080, 9000]);

    for candidate in candidates {
        if used.insert(candidate) {
            return Some(candidate);
        }
    }

    let mut fallback = 10_000_u16.saturating_add(index as u16);
    while used.contains(&fallback) && fallback < u16::MAX {
        fallback = fallback.saturating_add(1);
    }
    used.insert(fallback);
    Some(fallback)
}
