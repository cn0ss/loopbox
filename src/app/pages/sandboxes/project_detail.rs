use super::*;

#[component]
pub(super) fn ProjectDetail(
    project_name: String,
    project: ProjectConfig,
    suffix_preview: String,
    mut config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut selected_project: Signal<Option<String>>,
    pending_auto_apply: Signal<Option<String>>,
    mut runtime_tick: Signal<u64>,
) -> Element {
    // ── Local State ──
    let mut active_tab = use_signal(|| DetailTab::Services);
    let initial_log_filter = project.services.first().map(|service| service.name.clone());
    let mut log_filter = use_signal(move || initial_log_filter.clone());
    let mut env_editing_path = use_signal(|| None::<String>);
    let mut env_editing_content = use_signal(String::new);
    let mut env_editing_original = use_signal(String::new);
    let mut env_search_open = use_signal(|| false);
    let mut env_search_query = use_signal(String::new);
    let mut env_search_index = use_signal(|| 0_usize);
    let mut service_key_inputs = use_signal(BTreeMap::<String, String>::new);

    let initial_project = project.clone();
    let mut edit_form = use_signal(move || ProjectEditForm::from_project(&initial_project));
    let mut proxy_endpoints_form = use_signal(|| project.proxy_endpoints.clone());
    let mut grpc_proto_paths_form = use_signal(|| project.grpc_proto_paths.join("\n"));

    // ── Derived: Runtime Status (reactive via tick) ──
    let pn_status = project_name.clone();
    let runtime_status = use_memo(move || {
        let _tick = runtime_tick();
        let cfg = config();
        let mut statuses = BTreeMap::new();
        if let Some(proj) = cfg.projects.get(&pn_status) {
            for service in &proj.services {
                if let Ok(status) = loopbox::service_runtime_status(&cfg, &pn_status, &service.name)
                {
                    statuses.insert(service.name.clone(), status);
                }
            }
        }
        statuses
    });

    // ── Derived: Live Logs (reactive via tick) ──
    let pn_logs = project_name.clone();
    let live_logs = use_memo(move || {
        let _tick = runtime_tick();
        let filter = log_filter();
        let cfg = config();
        let mut logs: Vec<(String, String)> = Vec::new();
        if let Some(proj) = cfg.projects.get(&pn_logs) {
            let selected_service = effective_log_selection(proj.services.as_slice(), filter);
            let Some(selected_service) = selected_service else {
                return logs;
            };
            for service in &proj.services {
                if selected_service != service.name {
                    continue;
                }
                if let Ok(lines) = loopbox::service_logs(&pn_logs, &service.name) {
                    for line in lines {
                        logs.push((service.name.clone(), line));
                    }
                }
            }
        }
        logs
    });
    let pn_log_count = project_name.clone();
    let total_log_count = use_memo(move || {
        let _tick = runtime_tick();
        let cfg = config();
        let mut count = 0_usize;
        if let Some(proj) = cfg.projects.get(&pn_log_count) {
            for service in &proj.services {
                if let Ok(lines) = loopbox::service_logs(&pn_log_count, &service.name) {
                    count = count.saturating_add(lines.len());
                }
            }
        }
        count
    });

    // ── Snapshots ──
    let tab = active_tab();
    let status_snapshot = runtime_status();
    let service_key_inputs_snapshot = service_key_inputs();
    let logs_snapshot = live_logs();
    let total_logs_snapshot = total_log_count();
    let selected_log_service = effective_log_selection(project.services.as_slice(), log_filter());
    let log_attached = selected_log_service
        .as_ref()
        .and_then(|service| loopbox::service_log_attached(&project_name, service).ok())
        .unwrap_or(false);
    let show_traffic_tab = project_detail_show_traffic_tab(&config(), &project_name);
    let grpc_proto_decode_enabled = true;
    let edit_snapshot = edit_form();
    let proxy_endpoints_snapshot = proxy_endpoints_form();
    let grpc_proto_paths_snapshot = grpc_proto_paths_form();

    // Status counts
    let running_count = status_snapshot
        .values()
        .filter(|s| s.state == ServiceRuntimeState::Running)
        .count();
    let total_services = project.services.len();

    // Edit form state
    let edit_dirty = edit_snapshot != ProjectEditForm::from_project(&project);
    let proxy_endpoints_dirty = proxy_endpoints_snapshot != project.proxy_endpoints;
    let grpc_proto_paths_dirty =
        normalize_grpc_proto_paths_for_form(&grpc_proto_paths_snapshot) != project.grpc_proto_paths;
    let edit_suffix = preview_suffix(&suffix_preview);
    let mut edit_host_previews = Vec::new();
    for entry in &edit_snapshot.services {
        let svc_name = preview_service_name(&entry.name);
        if svc_name.is_empty() {
            continue;
        }
        let host = format!("{svc_name}.{project_name}.{edit_suffix}");
        if !edit_host_previews.contains(&host) {
            edit_host_previews.push(host);
        }
    }

    // Env editor state
    let env_path = env_editing_path();
    let env_content = env_editing_content();
    let env_original = env_editing_original();
    let env_dirty = env_path.is_some() && env_content != env_original;
    let search_open = env_search_open();
    let search_query = env_search_query();
    let search_idx = env_search_index();

    // Set up editor scroll sync + tab handling via trusted webview JS
    use_effect(move || {
        let path = env_editing_path();
        if path.is_some() {
            run_webview_js(
                r#"setTimeout(function() {
                    var ta = document.getElementById('env-editor-input');
                    var hl = document.getElementById('env-editor-highlight');
                    var gt = document.getElementById('env-editor-gutter');
                    if (!ta || !hl || !gt) return;
                    if (ta._lbScroll) ta.removeEventListener('scroll', ta._lbScroll);
                    ta._lbScroll = function() {
                        hl.scrollTop = ta.scrollTop;
                        hl.scrollLeft = ta.scrollLeft;
                        gt.scrollTop = ta.scrollTop;
                    };
                    ta.addEventListener('scroll', ta._lbScroll);
                    if (ta._lbTab) ta.removeEventListener('keydown', ta._lbTab);
                    ta._lbTab = function(e) {
                        if (e.key === 'Tab') {
                            e.preventDefault();
                            e.stopPropagation();
                            var s = ta.selectionStart, end = ta.selectionEnd;
                            ta.value = ta.value.substring(0, s) + '    ' + ta.value.substring(end);
                            ta.selectionStart = ta.selectionEnd = s + 4;
                            ta.dispatchEvent(new Event('input', { bubbles: true }));
                        }
                    };
                    ta.addEventListener('keydown', ta._lbTab);
                }, 30);"#,
            );
        }
    });

    // Env exports
    let env_preview = loopbox::project_env_exports(&config(), &project_name)
        .unwrap_or_else(|err| format!("# {err}"));
    let run_hint = format!("cd {}\neval \"$(loopbox env)\"\nmake dev", project.dir);

    // Force refresh helper
    let mut force_tick = move || {
        runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
    };

    // Helper: focus search input and scroll to match position
    fn focus_search_at(start: usize, end: usize) {
        let js = format!(
            "var t=document.getElementById('env-editor-input');if(t){{t.focus();t.setSelectionRange({start},{end});var lh=parseFloat(getComputedStyle(t).lineHeight)||16;var ln=t.value.substring(0,{start}).split('\\n').length-1;t.scrollTop=ln*lh-t.clientHeight/3;}}"
        );
        run_webview_js(&js);
    }

    let mut tab_items = vec![
        (DetailTab::Services, "Services"),
        (DetailTab::Logs, "Logs"),
        (DetailTab::Environment, "Environment"),
        (DetailTab::Config, "Config"),
    ];
    if show_traffic_tab {
        tab_items.insert(2, (DetailTab::Traffic, "Traffic"));
    }

    rsx! {
        div { class: "page",
            // ── Breadcrumb / Back ──
            div { class: "detail-breadcrumb",
                button {
                    class: "breadcrumb-link",
                    onclick: move |_| selected_project.set(None),
                    "Sandboxes"
                }
                span { class: "breadcrumb-sep", "/" }
                span { class: "breadcrumb-current", "{project_name}" }
            }

            // ── Header ──
            div { class: "detail-header",
                div { class: "detail-header-left",
                    h1 { class: "page-title", "{project_name}" }
                    span { class: "detail-ip", "{project.ip}" }
                    if running_count > 0 {
                        span { class: "detail-status detail-status-active",
                            span { class: "status-dot status-dot-running" }
                            "{running_count}/{total_services} running"
                        }
                    } else {
                        span { class: "detail-status", "idle" }
                    }
                }
            }

            // ── Tab Bar ──
            div { class: "tab-bar",
                for (tab_value, label) in tab_items {
                    button {
                        key: "{label}",
                        class: if tab == tab_value { "tab-item tab-item-active" } else { "tab-item" },
                        onclick: move |_| active_tab.set(tab_value),
                        "{label}"
                        if tab_value == DetailTab::Services && running_count > 0 {
                            span { class: "tab-badge tab-badge-ok", "{running_count}" }
                        }
                        if tab_value == DetailTab::Logs && total_logs_snapshot > 0 {
                            span { class: "tab-badge", "{total_logs_snapshot}" }
                        }
                    }
                }
            }

            // ════════════════════════════════════════
            // TAB: Services
            // ════════════════════════════════════════
            if tab == DetailTab::Services {
                div { class: "tab-content",
                    div { class: "svc-toolbar",
                        div { class: "svc-toolbar-status",
                            for (state_name, count) in service_status_summary(&status_snapshot) {
                                if count > 0 {
                                    span { class: "svc-status-chip", key: "{state_name}", "{count} {state_name}" }
                                }
                            }
                        }
                        div { class: "svc-toolbar-actions",
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let pn = project_name.clone();
                                    move |_| {
                                        match loopbox::start_project_all(&config(), &pn) {
                                            Ok(results) => {
                                                notice.set(Some(Notice::success(format!(
                                                    "Started {} service(s).",
                                                    results.len()
                                                ))));
                                                force_tick();
                                            }
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                },
                                "Start All"
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let pn = project_name.clone();
                                    move |_| {
                                        match loopbox::stop_project_all(&config(), &pn) {
                                            Ok(results) => {
                                                notice.set(Some(Notice::info(format!(
                                                    "Stopped {} service(s).",
                                                    results.len()
                                                ))));
                                                force_tick();
                                            }
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                },
                                "Stop All"
                            }
                        }
                    }

                    div { class: "svc-list",
                        for service in &project.services {
                            {{
                                let runtime = status_snapshot.get(&service.name);
                                let (state_label, dot_class) = runtime_badge(runtime);
                                let border_class = svc_card_border_class(runtime);
                                let effective_ports = loopbox::service_ports(service);
                                let service_has_open_url = effective_ports
                                    .iter()
                                    .any(|entry| entry.protocol == ProxyEndpointProtocol::Http1);
                                let default_service_port_label = if effective_ports.is_empty() {
                                    "—".to_string()
                                } else {
                                    effective_ports
                                        .iter()
                                        .map(|entry| {
                                            format!(
                                                ":{}/{}",
                                                entry.port,
                                                service_protocol_value(&entry.protocol)
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                };
                                let service_port_label = project_detail_service_port_label_override(
                                    &project.ip,
                                    service,
                                )
                                .unwrap_or(default_service_port_label);
                                let runtime_label = project_detail_service_runtime_label(service);
                                let execution_label = project_detail_service_execution_label(service)
                                    .unwrap_or_else(|| format!("$ {}", service.command));
                                let is_running = runtime.is_some_and(|s| {
                                    matches!(
                                        s.state,
                                        ServiceRuntimeState::Running
                                            | ServiceRuntimeState::Starting
                                            | ServiceRuntimeState::Unhealthy
                                    )
                                });
                                let is_process_runtime =
                                    matches!(service.runtime, crate::loopbox::ServiceRuntimeKind::Process);
                                let input_attached = is_running
                                    && is_process_runtime
                                    && loopbox::service_input_attached(&project_name, &service.name)
                                        .unwrap_or(false);
                                let key_input_value = service_key_inputs_snapshot
                                    .get(&service.name)
                                    .cloned()
                                    .unwrap_or_default();

                                rsx! {
                                    div { class: "svc-card {border_class}", key: "{service.name}",
                                        div { class: "svc-card-main",
                                            div { class: "svc-card-identity",
                                                span { class: "svc-card-dot {dot_class}" }
                                                span { class: "svc-card-name", "{service.name}" }
                                                if let Some(runtime_label) = runtime_label {
                                                    span { class: "svc-card-state", "{runtime_label}" }
                                                }
                                                span { class: "svc-card-port",
                                                    "{service_port_label}"
                                                }
                                            }
                                            div { class: "svc-card-meta",
                                                span { class: "svc-card-state", "{state_label}" }
                                                if let Some(snapshot) = runtime {
                                                    if let Some(pid) = snapshot.pid {
                                                        span { class: "svc-card-pid", "pid {pid}" }
                                                    }
                                                    if let Some(code) = snapshot.exit_code {
                                                        span { class: "svc-card-exit", "exit {code}" }
                                                    }
                                                }
                                            }
                                        }
                                        div { class: "svc-card-cmd", "{execution_label}" }
                                        div { class: "svc-card-actions",
                                            if is_running {
                                                button {
                                                    class: "btn btn-sm btn-outline",
                                                    onclick: {
                                                        let pn = project_name.clone();
                                                        let svc = service.name.clone();
                                                        move |_| {
                                                            match loopbox::stop_service(&pn, &svc) {
                                                                Ok(_) => {
                                                                    notice.set(Some(Notice::info(format!("Stopped '{svc}'."))));
                                                                    force_tick();
                                                                }
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    },
                                                    "Stop"
                                                }
                                                button {
                                                    class: "btn btn-sm btn-outline",
                                                    onclick: {
                                                        let pn = project_name.clone();
                                                        let svc = service.name.clone();
                                                        move |_| {
                                                            match loopbox::restart_service(&config(), &pn, &svc) {
                                                                Ok(_) => {
                                                                    notice.set(Some(Notice::success(format!("Restarted '{svc}'."))));
                                                                    force_tick();
                                                                }
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    },
                                                    "Restart"
                                                }
                                                if is_process_runtime {
                                                    if input_attached {
                                                        button {
                                                            class: "btn btn-sm btn-outline",
                                                            onclick: {
                                                                let pn = project_name.clone();
                                                                let svc = service.name.clone();
                                                                move |_| {
                                                                    match loopbox::open_terminal_attach_for_service(&pn, &svc) {
                                                                        Ok(msg) => notice.set(Some(Notice::info(msg))),
                                                                        Err(err) => notice.set(Some(Notice::error(err))),
                                                                    }
                                                                }
                                                            },
                                                            "Attach"
                                                        }
                                                    }
                                                    if input_attached {
                                                        div { class: "svc-input-control",
                                                            input {
                                                                class: "field-input svc-key-input",
                                                                r#type: "text",
                                                                value: "{key_input_value}",
                                                                placeholder: "Keys (example: r, \\n, \\x12)",
                                                                oninput: {
                                                                    let svc = service.name.clone();
                                                                    move |evt| {
                                                                        let value = evt.value();
                                                                        service_key_inputs.with_mut(|inputs| {
                                                                            if value.is_empty() {
                                                                                inputs.remove(&svc);
                                                                            } else {
                                                                                inputs.insert(svc.clone(), value);
                                                                            }
                                                                        });
                                                                    }
                                                                },
                                                            }
                                                            button {
                                                                class: "btn btn-sm btn-outline",
                                                                onclick: {
                                                                    let pn = project_name.clone();
                                                                    let svc = service.name.clone();
                                                                    move |_| {
                                                                        let raw_input = service_key_inputs
                                                                            .read()
                                                                            .get(&svc)
                                                                            .cloned()
                                                                            .unwrap_or_default();
                                                                        if raw_input.is_empty() {
                                                                            notice.set(Some(Notice::error("Enter one or more keys to send.".to_string())));
                                                                            return;
                                                                        }
                                                                        let decoded = match decode_service_input_sequence(&raw_input) {
                                                                            Ok(value) => value,
                                                                            Err(err) => {
                                                                                notice.set(Some(Notice::error(err)));
                                                                                return;
                                                                            }
                                                                        };
                                                                        match loopbox::send_service_input(&pn, &svc, &decoded) {
                                                                            Ok(()) => notice.set(Some(Notice::info(format!(
                                                                                "Sent key input to '{svc}'."
                                                                            )))),
                                                                            Err(err) => notice.set(Some(Notice::error(err))),
                                                                        }
                                                                    }
                                                                },
                                                                "Send Key"
                                                            }
                                                        }
                                                    } else {
                                                        span {
                                                            class: "svc-input-hint",
                                                            "Key input unavailable for this process instance. Restart it from here to re-attach stdin."
                                                        }
                                                    }
                                                }
                                            } else {
                                                button {
                                                    class: "btn btn-sm btn-primary",
                                                    onclick: {
                                                        let pn = project_name.clone();
                                                        let svc = service.name.clone();
                                                        move |_| {
                                                            match loopbox::start_service(&config(), &pn, &svc) {
                                                                Ok(_) => {
                                                                    notice.set(Some(Notice::success(format!("Started '{svc}'."))));
                                                                    force_tick();
                                                                }
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    },
                                                    "Start"
                                                }
                                                button {
                                                    class: "btn btn-sm btn-outline",
                                                    onclick: {
                                                        let pn = project_name.clone();
                                                        let svc = service.name.clone();
                                                        move |_| {
                                                            match loopbox::open_terminal_for_service(&config(), &pn, &svc, false) {
                                                                Ok(msg) => notice.set(Some(Notice::info(msg))),
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    },
                                                    "Terminal"
                                                }
                                                button {
                                                    class: "btn btn-sm btn-outline",
                                                    onclick: {
                                                        let pn = project_name.clone();
                                                        let svc = service.name.clone();
                                                        move |_| {
                                                            match loopbox::open_terminal_for_service(&config(), &pn, &svc, true) {
                                                                Ok(msg) => notice.set(Some(Notice::success(msg))),
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    },
                                                    "Run"
                                                }
                                            }
                                            if service_has_open_url {
                                                button {
                                                    class: "btn btn-sm btn-outline",
                                                    onclick: {
                                                        let pn = project_name.clone();
                                                        let svc = service.name.clone();
                                                        move |_| {
                                                            match loopbox::open_url_for(&config(), &pn, OpenTarget::Service(svc.clone())) {
                                                                Ok(url) => match webbrowser::open(&url) {
                                                                    Ok(_) => notice.set(Some(Notice::info(format!("Opened {url}")))),
                                                                    Err(err) => notice.set(Some(Notice::error(format!("Failed: {err}")))),
                                                                },
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    },
                                                    "Open"
                                                }
                                            }
                                        }
                                    }
                                }
                            }}
                        }
                    }
                }
            }

            if show_traffic_tab && tab == DetailTab::Traffic {
                ProjectDetailEeTrafficTab {
                    project_name: project_name.clone(),
                    project: project.clone(),
                    config,
                    notice,
                    runtime_tick,
                }
            }

            // ════════════════════════════════════════
            // TAB: Logs
            // ════════════════════════════════════════
            if tab == DetailTab::Logs {
                div { class: "tab-content tab-content-logs",
                    div { class: "log-toolbar",
                        div { class: "log-toolbar-filters",
                            for service in &project.services {
                                button {
                                    key: "{service.name}",
                                    class: if selected_log_service.as_ref() == Some(&service.name) {
                                        "btn btn-sm btn-toggle-on"
                                    } else {
                                        "btn btn-sm btn-outline"
                                    },
                                    onclick: {
                                        let svc = service.name.clone();
                                        move |_| log_filter.set(Some(svc.clone()))
                                    },
                                    "{service.name}"
                                }
                            }
                        }
                        div { class: "log-toolbar-right",
                            if let Some(_selected_service) = selected_log_service.as_ref() {
                                span {
                                    class: if log_attached { "log-status log-status-attached" } else { "log-status" },
                                    if log_attached { "attached" } else { "detached" }
                                }
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let pn = project_name.clone();
                                    let selected_service = selected_log_service.clone();
                                    move |_| {
                                        if let Some(selected_service) = selected_service.clone() {
                                            let _ = loopbox::clear_service_logs(&pn, &selected_service);
                                            notice.set(Some(Notice::info(format!(
                                                "Cleared logs for '{}'.",
                                                selected_service
                                            ))));
                                            force_tick();
                                        } else {
                                            notice.set(Some(Notice::info("No service selected.".to_string())));
                                        }
                                    }
                                },
                                "Clear"
                            }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let pn = project_name.clone();
                                    let svc = selected_log_service.clone().unwrap_or_default();
                                    move |_| {
                                        let pn = pn.clone();
                                        let svc = svc.clone();
                                        let title = format!("Logs — {svc} ({pn})");
                                        log_window::push_config(LogWindowConfig {
                                            project_name: pn,
                                            service_name: svc,
                                        });
                                        let dom = VirtualDom::new(log_window::LogPopoutWindow);
                                        let cfg = dioxus::desktop::Config::new().with_window(
                                            dioxus::desktop::WindowBuilder::new()
                                                .with_title(title)
                                                .with_inner_size(dioxus::desktop::LogicalSize::new(900.0, 600.0)),
                                        );
                                        dioxus::desktop::window().new_window(dom, cfg);
                                    }
                                },
                                "\u{29C9} Pop Out"
                            }
                        }
                    }

                    div { class: "log-viewer-wrap",
                        div { id: "log-viewer-main", class: "log-viewer",
                            if logs_snapshot.is_empty() {
                                div { class: "log-empty",
                                    p { "No log output yet. Start a service to see logs here." }
                                    p { class: "log-empty-hint", "Logs auto-refresh every second." }
                                }
                            } else {
                                for (idx, (service_name, line)) in logs_snapshot.iter().enumerate() {
                                    div { class: log_line_outer_class(line), key: "{service_name}-{idx}",
                                        span { class: "log-svc", "{service_name}" }
                                        span { class: log_line_text_class(line), "{strip_log_prefix(line)}" }
                                    }
                                }
                            }
                        }
                        button {
                            id: "log-jump-main",
                            class: "log-jump-btn",
                            onclick: move |_| {
                                run_webview_js(
                                    "var el=document.getElementById('log-viewer-main');\
                                     if(el){el.scrollTop=el.scrollHeight;el._tailing=true;\
                                     var b=document.getElementById('log-jump-main');\
                                     if(b)b.style.display='none';}"
                                );
                            },
                            span { class: "log-jump-btn-arrow", "\u{2193}" }
                            "Jump to latest"
                        }
                    }
                    {
                        // Auto-scroll: install scroll listener + scroll if tailing
                        run_webview_js(&format!(
                            "(function(){{\
                                var el=document.getElementById('log-viewer-main');\
                                if(!el)return;\
                                if(!el._scrollSetup){{\
                                    el._scrollSetup=true;el._tailing=true;\
                                    el.addEventListener('scroll',function(){{\
                                        el._tailing=el.scrollTop+el.clientHeight>=el.scrollHeight-50;\
                                        var b=document.getElementById('log-jump-main');\
                                        if(b)b.style.display=el._tailing?'none':'flex';\
                                    }});\
                                }}\
                                if(el._tailing)el.scrollTop=el.scrollHeight;\
                            }})();/* t={} */",
                            total_logs_snapshot
                        ));
                        rsx! {}
                    }
                }
            }

            // ════════════════════════════════════════
            // TAB: Environment
            // ════════════════════════════════════════
            if tab == DetailTab::Environment {
                div { class: "tab-content",
                    section { class: "panel",
                        div { class: "panel-header",
                            h2 { "Discovered Files" }
                        }
                        match loopbox::discover_env_files(&project.dir) {
                            Ok(files) => {
                                if files.is_empty() {
                                    rsx! { p { class: "text-dim", "No .env files found in project directory." } }
                                } else {
                                    rsx! {
                                        div { class: "env-file-list",
                                            for file in files {
                                                {{
                                                    let display_path = file.strip_prefix(&format!("{}/", project.dir))
                                                        .unwrap_or(&file)
                                                        .to_string();
                                                    let is_active = env_path.as_ref() == Some(&file);
                                                    rsx! {
                                                        div {
                                                            class: if is_active { "env-file-item env-file-item-active" } else { "env-file-item" },
                                                            key: "{file}",
                                                            span { class: "env-file-path", "{display_path}" }
                                                            button {
                                                                class: "btn btn-sm btn-outline",
                                                                onclick: {
                                                                    let path = file.clone();
                                                                    move |_| {
                                                                        match loopbox::read_env_file_content(&path) {
                                                                            Ok(content) => {
                                                                                env_editing_path.set(Some(path.clone()));
                                                                                env_editing_content.set(content.clone());
                                                                                env_editing_original.set(content);
                                                                            }
                                                                            Err(err) => notice.set(Some(Notice::error(err))),
                                                                        }
                                                                    }
                                                                },
                                                                if is_active { "Reload" } else { "Edit" }
                                                            }
                                                        }
                                                    }
                                                }}
                                            }
                                        }
                                    }
                                }
                            }
                            Err(err) => rsx! { p { class: "text-dim", "{err}" } },
                        }
                    }

                    // Code editor
                    if let Some(ref editing_path) = env_path {
                        section { class: "panel",
                            div { class: "code-editor",

                                // ── Toolbar ──
                                div { class: "code-editor-toolbar",
                                    div { class: "code-editor-toolbar-left",
                                        span { class: "code-editor-file-badge",
                                            "{editing_path.rsplit('/').next().unwrap_or(editing_path)}"
                                        }
                                        if env_dirty {
                                            span { class: "code-editor-dirty-dot" }
                                            span { class: "code-editor-dirty-label", "modified" }
                                        }
                                    }
                                    div { class: "code-editor-toolbar-right",
                                        button {
                                            class: "code-editor-toolbar-btn",
                                            title: "Find (Cmd+F)",
                                            onclick: move |_| {
                                                let opening = !search_open;
                                                env_search_open.set(opening);
                                                if !opening {
                                                    env_search_query.set(String::new());
                                                    env_search_index.set(0);
                                                }
                                            },
                                            if search_open { "\u{2715} Find" } else { "\u{2315} Find" }
                                        }
                                        button {
                                            class: "btn btn-primary btn-sm",
                                            disabled: !env_dirty,
                                            onclick: {
                                                let path = editing_path.clone();
                                                move |_| {
                                                    match loopbox::write_env_file_content(&path, &env_editing_content()) {
                                                        Ok(()) => {
                                                            env_editing_original.set(env_editing_content());
                                                            notice.set(Some(Notice::success(format!("Saved {}.", path))));
                                                        }
                                                        Err(err) => notice.set(Some(Notice::error(err))),
                                                    }
                                                }
                                            },
                                            "Save"
                                        }
                                        button {
                                            class: "btn btn-outline btn-sm",
                                            disabled: !env_dirty,
                                            onclick: move |_| {
                                                env_editing_content.set(env_editing_original());
                                            },
                                            "Revert"
                                        }
                                        button {
                                            class: "btn btn-outline btn-sm",
                                            onclick: move |_| {
                                                env_editing_path.set(None);
                                                env_editing_content.set(String::new());
                                                env_editing_original.set(String::new());
                                                env_search_open.set(false);
                                                env_search_query.set(String::new());
                                            },
                                            "Close"
                                        }
                                    }
                                }

                                // ── Search Panel ──
                                if search_open {
                                    {{
                                        let match_count = env_search_match_count(&env_content, &search_query);
                                        let display_idx = if match_count > 0 { search_idx + 1 } else { 0 };
                                        rsx! {
                                            div { class: "code-editor-search",
                                                input {
                                                    class: "code-editor-search-input",
                                                    r#type: "text",
                                                    placeholder: "Find\u{2026}",
                                                    value: "{search_query}",
                                                    oninput: move |evt| {
                                                        env_search_query.set(evt.value());
                                                        env_search_index.set(0);
                                                    },
                                                    onkeydown: move |evt| {
                                                        if evt.key() == Key::Enter {
                                                            let total = env_search_match_count(&env_editing_content(), &env_search_query());
                                                            if total > 0 {
                                                                let next = (env_search_index() + 1) % total;
                                                                env_search_index.set(next);
                                                                if let Some((start, end)) = env_search_match_offset(&env_editing_content(), &env_search_query(), next) {
                                                                    focus_search_at(start, end);
                                                                }
                                                            }
                                                        }
                                                        if evt.key() == Key::Escape {
                                                            env_search_open.set(false);
                                                            env_search_query.set(String::new());
                                                            env_search_index.set(0);
                                                        }
                                                    },
                                                }
                                                span {
                                                    class: if match_count > 0 { "code-editor-search-count code-editor-search-count-active" } else { "code-editor-search-count" },
                                                    if search_query.is_empty() {
                                                        ""
                                                    } else {
                                                        "{display_idx} of {match_count}"
                                                    }
                                                }
                                                button {
                                                    class: "code-editor-search-btn",
                                                    disabled: match_count == 0,
                                                    title: "Previous match",
                                                    onclick: move |_| {
                                                        let total = env_search_match_count(&env_editing_content(), &env_search_query());
                                                        if total > 0 {
                                                            let prev = if env_search_index() == 0 { total - 1 } else { env_search_index() - 1 };
                                                            env_search_index.set(prev);
                                                            if let Some((start, end)) = env_search_match_offset(&env_editing_content(), &env_search_query(), prev) {
                                                                focus_search_at(start, end);
                                                            }
                                                        }
                                                    },
                                                    "\u{2191}"
                                                }
                                                button {
                                                    class: "code-editor-search-btn",
                                                    disabled: match_count == 0,
                                                    title: "Next match",
                                                    onclick: move |_| {
                                                        let total = env_search_match_count(&env_editing_content(), &env_search_query());
                                                        if total > 0 {
                                                            let next = (env_search_index() + 1) % total;
                                                            env_search_index.set(next);
                                                            if let Some((start, end)) = env_search_match_offset(&env_editing_content(), &env_search_query(), next) {
                                                                focus_search_at(start, end);
                                                            }
                                                        }
                                                    },
                                                    "\u{2193}"
                                                }
                                                button {
                                                    class: "code-editor-search-btn",
                                                    title: "Close search (Esc)",
                                                    onclick: move |_| {
                                                        env_search_open.set(false);
                                                        env_search_query.set(String::new());
                                                        env_search_index.set(0);
                                                    },
                                                    "\u{2715}"
                                                }
                                            }
                                        }
                                    }}
                                }

                                // ── Editor Body ──
                                div { class: "code-editor-body",
                                    div {
                                        class: "code-editor-gutter",
                                        id: "env-editor-gutter",
                                        {{
                                            let line_count = env_content.split('\n').count().max(1);
                                            rsx! {
                                                for i in 1..=line_count {
                                                    div {
                                                        class: "code-editor-line-num",
                                                        key: "{i}",
                                                        "{i}"
                                                    }
                                                }
                                            }
                                        }}
                                    }
                                    div { class: "code-editor-content",
                                        pre {
                                            class: "code-editor-highlight",
                                            id: "env-editor-highlight",
                                            dangerous_inner_html: "{highlight_env_content(&env_content)}",
                                        }
                                        textarea {
                                            class: "code-editor-input",
                                            id: "env-editor-input",
                                            value: "{env_content}",
                                            oninput: move |evt| env_editing_content.set(evt.value()),
                                            onkeydown: {
                                                let save_path = editing_path.clone();
                                                move |evt: KeyboardEvent| {
                                                    if (evt.modifiers().contains(Modifiers::META) || evt.modifiers().contains(Modifiers::CONTROL))
                                                        && evt.key() == Key::Character("s".to_string())
                                                    {
                                                        evt.prevent_default();
                                                        let dirty = env_editing_content() != env_editing_original();
                                                        if dirty {
                                                            match loopbox::write_env_file_content(&save_path, &env_editing_content()) {
                                                                Ok(()) => {
                                                                    env_editing_original.set(env_editing_content());
                                                                    notice.set(Some(Notice::success(format!("Saved {}.", save_path))));
                                                                }
                                                                Err(err) => notice.set(Some(Notice::error(err))),
                                                            }
                                                        }
                                                    }
                                                    if (evt.modifiers().contains(Modifiers::META) || evt.modifiers().contains(Modifiers::CONTROL))
                                                        && evt.key() == Key::Character("f".to_string())
                                                    {
                                                        evt.prevent_default();
                                                        env_search_open.set(true);
                                                        run_webview_js("setTimeout(function(){var e=document.querySelector('.code-editor-search-input');if(e)e.focus();},30);");
                                                    }
                                                    if evt.key() == Key::Escape && env_search_open() {
                                                        env_search_open.set(false);
                                                        env_search_query.set(String::new());
                                                        env_search_index.set(0);
                                                    }
                                                }
                                            },
                                            spellcheck: "false",
                                        }
                                    }
                                }

                                // ── Status Bar ──
                                {{
                                    let line_count = env_content.split('\n').count();
                                    let char_count = env_content.len();
                                    let file_ext = editing_path.rsplit('.').next().unwrap_or("env");
                                    rsx! {
                                        div { class: "code-editor-status",
                                            span { class: "code-editor-status-item", "{line_count} lines" }
                                            span { class: "code-editor-status-item", "{char_count} chars" }
                                            span { class: "code-editor-status-item", "UTF-8" }
                                            span { class: "code-editor-status-item", ".{file_ext}" }
                                            if env_dirty {
                                                span { class: "code-editor-status-item code-editor-status-modified", "\u{25CF} modified" }
                                            }
                                        }
                                    }
                                }}
                            }
                        }
                    }

                    // Merged env per service
                    section { class: "panel",
                        div { class: "panel-header",
                            h2 { "Merged Environment" }
                        }
                        for service in &project.services {
                            {{
                                let merged = loopbox::merge_service_env(&config(), &project_name, &service.name);
                                rsx! {
                                    div { class: "env-merged-section", key: "{service.name}",
                                        h3 { "{service.name}" }
                                        if let Ok(result) = merged {
                                            if result.values.is_empty() {
                                                p { class: "text-dim", "No environment variables." }
                                            } else {
                                                if !result.overrides.is_empty() {
                                                    p { class: "text-dim",
                                                        "{result.overrides.len()} override(s) detected."
                                                    }
                                                }
                                                if !result.warnings.is_empty() {
                                                    p { class: "text-dim",
                                                        "{result.warnings.len()} parsing warning(s)."
                                                    }
                                                }
                                                div { class: "env-merged-grid",
                                                    for (key, value) in &result.values {
                                                        {{
                                                            let source = result
                                                                .sources
                                                                .get(key)
                                                                .cloned()
                                                                .unwrap_or_else(|| "unknown".to_string());
                                                            let source_label = compact_env_source(&project.dir, &source);
                                                            let display_value = redact_env_value(key, value);
                                                            rsx! {
                                                                div { class: "env-merged-row", key: "{key}",
                                                                    span { class: "env-merged-key", "{key}" }
                                                                    span { class: "env-merged-val", "{display_value}" }
                                                                    span { class: "text-dim", "{source_label}" }
                                                                    button {
                                                                        class: "btn btn-sm btn-outline env-merged-copy",
                                                                        onclick: {
                                                                            let kv = format!("{key}={value}");
                                                                            let k = key.clone();
                                                                            move |_| {
                                                                                match copy_to_clipboard(&kv) {
                                                                                    Ok(()) => notice.set(Some(Notice::success(format!("Copied '{k}'")))),
                                                                                    Err(err) => notice.set(Some(Notice::error(err))),
                                                                                }
                                                                            }
                                                                        },
                                                                        "Copy"
                                                                    }
                                                                }
                                                            }
                                                        }}
                                                    }
                                                }
                                            }
                                        } else {
                                            p { class: "text-dim", "Failed to load env." }
                                        }
                                    }
                                }
                            }}
                        }
                    }
                }
            }

            // ════════════════════════════════════════
            // TAB: Config
            // ════════════════════════════════════════
            if tab == DetailTab::Config {
                div { class: "tab-content",
                    section { class: "panel",
                        div { class: "panel-header",
                            h2 { "Sandbox Configuration" }
                            if edit_dirty {
                                span { class: "dirty-badge", "\u{25CF} unsaved" }
                            }
                        }

                        div { class: "field-grid",
                            label { class: "field field-wide",
                                span { "Directory" }
                                input {
                                    value: "{edit_snapshot.dir}",
                                    placeholder: "/Users/you/dev/project",
                                    oninput: move |evt| edit_form.write().dir = evt.value(),
                                }
                            }
                            label { class: "field",
                                span { "IP Address" }
                                input {
                                    value: "{edit_snapshot.ip}",
                                    placeholder: "127.0.0.X",
                                    oninput: move |evt| edit_form.write().ip = evt.value(),
                                }
                            }
                            div { class: "field field-wide field-generated",
                                span { "Generated Hosts" }
                                div { class: "field-generated-lines",
                                    if edit_host_previews.is_empty() {
                                        p { class: "field-generated-line field-generated-dim",
                                            "Add named services to generate hostnames."
                                        }
                                    } else {
                                        for host in &edit_host_previews {
                                            p { class: "field-generated-line", key: "{host}", "{host}" }
                                        }
                                    }
                                }
                            }
                        }

                        div { class: "service-section",
                            div { class: "service-section-header",
                                span { "Services" }
                                div { class: "service-section-actions",
                                    button {
                                        class: "btn btn-sm btn-outline",
                                        onclick: {
                                            let pn = project_name.clone();
                                            move |_| {
                                                let cfg = config();
                                                let Some(project_snapshot) = cfg.projects.get(&pn) else {
                                                    notice.set(Some(Notice::error(format!("Project '{pn}' not found."))));
                                                    return;
                                                };
                                                match loopbox::discover_project_commands(&project_snapshot.dir) {
                                                    Ok(suggestions) => {
                                                        let mut updated = edit_form();
                                                        let mut matched = 0_usize;
                                                        for service in &mut updated.services {
                                                            if service.name.trim().is_empty() {
                                                                continue;
                                                            }
                                                            if let Some(suggestion) =
                                                                loopbox::best_command_for_service(&service.name, &suggestions)
                                                            {
                                                                service.command = suggestion.command;
                                                                if service.workdir.trim().is_empty() {
                                                                    service.workdir = suggestion.workdir;
                                                                }
                                                                matched += 1;
                                                            }
                                                        }
                                                        edit_form.set(updated);
                                                        notice.set(Some(Notice::info(format!(
                                                            "Detected commands for {matched} service(s)."
                                                        ))));
                                                    }
                                                    Err(err) => notice.set(Some(Notice::error(err))),
                                                }
                                            }
                                        },
                                        "Auto Detect"
                                    }
                                    button {
                                        class: "btn btn-sm btn-outline",
                                        onclick: move |_| {
                                            edit_form
                                                .write()
                                                .services
                                                .push(wizard_blank_service_entry());
                                        },
                                        "+ Service"
                                    }
                                }
                            }
                            div { class: "service-edit-list",
                                for (i, entry) in edit_snapshot.services.iter().enumerate() {
                                    div { class: "service-edit-card", key: "{i}",
                                        div { class: "service-edit-head",
                                            span { class: "service-edit-num", "#{i + 1}" }
                                            if !entry.name.is_empty() {
                                                span { class: "service-edit-name", "{entry.name}" }
                                            }
                                            button {
                                                class: "service-edit-remove",
                                                onclick: move |_| {
                                                    edit_form.write().services.remove(i);
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
                                                        edit_form.write().services[i].name = evt.value();
                                                    },
                                                }
                                            }
                                            div { class: "field field-wide",
                                                span { "Ports (optional, multiple supported)" }
                                                div { class: "service-port-list",
                                                    for (port_idx, port_entry) in service_entry_port_rows(entry).iter().enumerate() {
                                                        div { class: "service-port-row", key: "edit-port-{i}-{port_idx}",
                                                            input {
                                                                value: "{port_entry.port}",
                                                                placeholder: "Port (e.g. 8080)",
                                                                oninput: move |evt: Event<FormData>| {
                                                                    edit_form.with_mut(|form| {
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
                                                                    edit_form.with_mut(|form| {
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
                                                                    edit_form.with_mut(|form| {
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
                                                                    edit_form.with_mut(|form| {
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
                                                        edit_form.with_mut(|form| {
                                                            if let Some(service) = form.services.get_mut(i) {
                                                                service.ports.push(blank_service_port_entry());
                                                                sync_service_entry_primary_port(service);
                                                            }
                                                        });
                                                    },
                                                    "+ Port"
                                                }
                                            }
                                            ProjectDetailEeServiceEditFields {
                                                service_index: i,
                                                entry: entry.clone(),
                                                edit_form,
                                                notice,
                                            }
                                            label { class: "field",
                                                span { "Workdir" }
                                                input {
                                                    value: "{entry.workdir}",
                                                    placeholder: "optional",
                                                    oninput: move |evt: Event<FormData>| {
                                                        edit_form.write().services[i].workdir = evt.value();
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Env Files" }
                                                input {
                                                    value: "{entry.env_files}",
                                                    placeholder: ".env,.env.local",
                                                    oninput: move |evt: Event<FormData>| {
                                                        edit_form.write().services[i].env_files = evt.value();
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Depends On" }
                                                input {
                                                    value: "{entry.depends_on}",
                                                    placeholder: "gateway,db",
                                                    oninput: move |evt: Event<FormData>| {
                                                        edit_form.write().services[i].depends_on = evt.value();
                                                    },
                                                }
                                            }
                                        }
                                        div { class: "service-edit-foot",
                                            button {
                                                class: if entry.autostart { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                                                onclick: move |_| {
                                                    let current = edit_form().services[i].autostart;
                                                    edit_form.write().services[i].autostart = !current;
                                                },
                                                if entry.autostart { "Autostart: on" } else { "Autostart: off" }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        div { class: "service-section",
                            div { class: "service-section-header",
                                span { "gRPC Proto Decode (Project Scope)" }
                                if !grpc_proto_decode_enabled {
                                    span { class: "text-dim", "Unavailable" }
                                }
                            }
                            p { class: "text-dim",
                                "Optional paths (one per line). Used to decode gRPC protobuf payloads in private builds."
                            }
                            if !grpc_proto_decode_enabled {
                                p { class: "text-dim",
                                    "gRPC proto decode is available in Ultimate."
                                }
                            }
                            label { class: "field field-wide",
                                span { "Proto paths" }
                                textarea {
                                    class: "field-input field-textarea",
                                    value: "{grpc_proto_paths_snapshot}",
                                    placeholder: "./proto\n./api/proto",
                                    disabled: !grpc_proto_decode_enabled,
                                    oninput: move |evt: Event<FormData>| {
                                        grpc_proto_paths_form.set(evt.value());
                                    },
                                }
                            }
                            div { class: "form-actions",
                                button {
                                    class: "btn btn-primary",
                                    disabled: !grpc_proto_decode_enabled || !grpc_proto_paths_dirty,
                                    onclick: {
                                        let pn = project_name.clone();
                                        move |_| {
                                            let parsed = normalize_grpc_proto_paths_for_form(&grpc_proto_paths_form());
                                            let previous = config();
                                            let mut project_found = false;
                                            config.with_mut(|cfg| {
                                                if let Some(project_cfg) = cfg.projects.get_mut(&pn) {
                                                    project_cfg.grpc_proto_paths = parsed.clone();
                                                    project_found = true;
                                                }
                                            });
                                            if !project_found {
                                                config.set(previous);
                                                notice.set(Some(Notice::error(format!(
                                                    "Project '{pn}' not found."
                                                ))));
                                                return;
                                            }

                                            let current = config();
                                            match loopbox::save_config(&current) {
                                                Ok(path) => {
                                                    match loopbox::load_config() {
                                                        Ok(reloaded) => {
                                                            let proto_paths_text = reloaded
                                                                .projects
                                                                .get(&pn)
                                                                .map(|project_cfg| project_cfg.grpc_proto_paths.join("\n"))
                                                                .unwrap_or_default();
                                                            grpc_proto_paths_form.set(proto_paths_text);
                                                            config.set(reloaded);
                                                        }
                                                        Err(err) => {
                                                            config.set(previous);
                                                            notice.set(Some(Notice::error(format!(
                                                                "Saved {}, but failed to reload config: {err}",
                                                                path.display()
                                                            ))));
                                                            return;
                                                        }
                                                    }

                                                    match loopbox::sync_reverse_proxy(&config()) {
                                                        Ok(_) => notice.set(Some(Notice::success(format!(
                                                            "gRPC proto paths updated for '{pn}'. Saved {}.",
                                                            path.display()
                                                        )))),
                                                        Err(err) => notice.set(Some(Notice::error(format!(
                                                            "Saved {}, but failed to apply proxy updates: {err}",
                                                            path.display()
                                                        )))),
                                                    }
                                                }
                                                Err(err) => {
                                                    config.set(previous);
                                                    notice.set(Some(Notice::error(err)));
                                                }
                                            }
                                        }
                                    },
                                    "Save Proto Paths"
                                }
                                button {
                                    class: "btn btn-outline",
                                    disabled: !grpc_proto_decode_enabled,
                                    onclick: {
                                        let pn = project_name.clone();
                                        move |_| {
                                            let current = config();
                                            let next = current
                                                .projects
                                                .get(&pn)
                                                .map(|project_cfg| project_cfg.grpc_proto_paths.join("\n"))
                                                .unwrap_or_default();
                                            grpc_proto_paths_form.set(next);
                                        }
                                    },
                                    "Reset Proto Paths"
                                }
                            }
                        }

                        div { class: "service-section",
                            div { class: "service-section-header",
                                span { "Advanced Proxy Endpoint Overrides" }
                                div { class: "service-section-actions",
                                    button {
                                        class: "btn btn-sm btn-outline",
                                        onclick: {
                                            let pn = project_name.clone();
                                            move |_| {
                                                proxy_endpoints_form.with_mut(|entries| {
                                                    entries.push(default_project_proxy_endpoint_config(&pn));
                                                });
                                            }
                                        },
                                        "+ Endpoint"
                                    }
                                }
                            }
                            p { class: "text-dim",
                                "Optional overrides for special cases. Normal gRPC routing is derived from service port + protocol."
                            }
                            if proxy_endpoints_snapshot.is_empty() {
                                p { class: "text-dim", "No advanced overrides configured for this sandbox." }
                            }
                            div { class: "service-edit-list",
                                for (i, endpoint) in proxy_endpoints_snapshot.iter().enumerate() {
                                    div { class: "service-edit-card", key: "proxy-endpoint-{i}",
                                        div { class: "service-edit-head",
                                            span { class: "service-edit-num", "#{i + 1}" }
                                            if !endpoint.name.trim().is_empty() {
                                                span { class: "service-edit-name", "{endpoint.name}" }
                                            }
                                            button {
                                                class: "service-edit-remove",
                                                onclick: move |_| {
                                                    proxy_endpoints_form.with_mut(|entries| {
                                                        if i < entries.len() {
                                                            entries.remove(i);
                                                        }
                                                    });
                                                },
                                                "×"
                                            }
                                        }
                                        div { class: "service-edit-grid",
                                            label { class: "field",
                                                span { "Name" }
                                                input {
                                                    value: "{endpoint.name}",
                                                    placeholder: "gateway-grpc",
                                                    oninput: move |evt: Event<FormData>| {
                                                        proxy_endpoints_form.with_mut(|entries| {
                                                            if let Some(entry) = entries.get_mut(i) {
                                                                entry.name = evt.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Protocol" }
                                                select {
                                                    value: "{project_proxy_endpoint_protocol_value(&endpoint.protocol)}",
                                                    onchange: move |evt: Event<FormData>| {
                                                        if let Some(next_protocol) =
                                                            parse_project_proxy_endpoint_protocol(&evt.value())
                                                        {
                                                            proxy_endpoints_form.with_mut(|entries| {
                                                                if let Some(entry) = entries.get_mut(i) {
                                                                    entry.protocol = next_protocol.clone();
                                                                    if entry.protocol != ProxyEndpointProtocol::GrpcH2c {
                                                                        entry.authority = None;
                                                                    }
                                                                }
                                                            });
                                                        }
                                                    },
                                                    option { value: "grpc_h2c", "grpc_h2c" }
                                                    option { value: "http1", "http1" }
                                                    option { value: "tcp_passthrough", "tcp_passthrough" }
                                                }
                                            }
                                            label { class: "field field-wide",
                                                span { "Authority (optional, gRPC)" }
                                                input {
                                                    value: "{endpoint.authority.clone().unwrap_or_default()}",
                                                    placeholder: "gateway.skybrid.localhost",
                                                    oninput: move |evt: Event<FormData>| {
                                                        proxy_endpoints_form.with_mut(|entries| {
                                                            if let Some(entry) = entries.get_mut(i) {
                                                                entry.authority =
                                                                    optional_trimmed_endpoint_value(&evt.value());
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Listen Host" }
                                                input {
                                                    value: "{endpoint.listen_host}",
                                                    placeholder: "127.0.0.1",
                                                    oninput: move |evt: Event<FormData>| {
                                                        proxy_endpoints_form.with_mut(|entries| {
                                                            if let Some(entry) = entries.get_mut(i) {
                                                                entry.listen_host = evt.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Listen Port" }
                                                input {
                                                    value: "{endpoint.listen_port}",
                                                    placeholder: "50051",
                                                    oninput: move |evt: Event<FormData>| {
                                                        if let Ok(port) = evt.value().trim().parse::<u16>() {
                                                            proxy_endpoints_form.with_mut(|entries| {
                                                                if let Some(entry) = entries.get_mut(i) {
                                                                    entry.listen_port = port;
                                                                }
                                                            });
                                                        }
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Upstream Host" }
                                                input {
                                                    value: "{endpoint.upstream_host}",
                                                    placeholder: "127.0.0.30",
                                                    oninput: move |evt: Event<FormData>| {
                                                        proxy_endpoints_form.with_mut(|entries| {
                                                            if let Some(entry) = entries.get_mut(i) {
                                                                entry.upstream_host = evt.value();
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Upstream Port" }
                                                input {
                                                    value: "{endpoint.upstream_port}",
                                                    placeholder: "50051",
                                                    oninput: move |evt: Event<FormData>| {
                                                        if let Ok(port) = evt.value().trim().parse::<u16>() {
                                                            proxy_endpoints_form.with_mut(|entries| {
                                                                if let Some(entry) = entries.get_mut(i) {
                                                                    entry.upstream_port = port;
                                                                }
                                                            });
                                                        }
                                                    },
                                                }
                                            }
                                            label { class: "field",
                                                span { "Service (optional)" }
                                                input {
                                                    value: "{endpoint.service_name.clone().unwrap_or_default()}",
                                                    placeholder: "gateway",
                                                    oninput: move |evt: Event<FormData>| {
                                                        proxy_endpoints_form.with_mut(|entries| {
                                                            if let Some(entry) = entries.get_mut(i) {
                                                                entry.service_name =
                                                                    optional_trimmed_endpoint_value(&evt.value());
                                                            }
                                                        });
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "form-actions",
                                button {
                                    class: "btn btn-primary",
                                    disabled: !proxy_endpoints_dirty,
                                    onclick: {
                                        let pn = project_name.clone();
                                        move |_| {
                                            let mut next_endpoints = proxy_endpoints_form();
                                            sanitize_project_proxy_endpoints_for_form(&pn, &mut next_endpoints);
                                            match validate_project_proxy_endpoints_for_form(&next_endpoints) {
                                                Ok(()) => {
                                                    let previous = config();
                                                    let mut project_found = false;
                                                    config.with_mut(|cfg| {
                                                        if let Some(project_cfg) = cfg.projects.get_mut(&pn) {
                                                            project_cfg.proxy_endpoints = next_endpoints.clone();
                                                            project_found = true;
                                                        }
                                                    });
                                                    if !project_found {
                                                        config.set(previous);
                                                        notice.set(Some(Notice::error(format!(
                                                            "Project '{pn}' not found."
                                                        ))));
                                                        return;
                                                    }

                                                    let current = config();
                                                    match loopbox::save_config(&current) {
                                                        Ok(path) => {
                                                            match loopbox::load_config() {
                                                                Ok(reloaded) => {
                                                                    let project_endpoints = reloaded
                                                                        .projects
                                                                        .get(&pn)
                                                                        .map(|project_cfg| project_cfg.proxy_endpoints.clone())
                                                                        .unwrap_or_default();
                                                                    proxy_endpoints_form.set(project_endpoints);
                                                                    config.set(reloaded);
                                                                }
                                                                Err(err) => {
                                                                    config.set(previous);
                                                                    notice.set(Some(Notice::error(format!(
                                                                        "Saved {}, but failed to reload config: {err}",
                                                                        path.display()
                                                                    ))));
                                                                    return;
                                                                }
                                                            }

                                                            match loopbox::sync_reverse_proxy(&config()) {
                                                                Ok(_) => notice.set(Some(Notice::success(format!(
                                                                    "Proxy endpoints updated for '{pn}'. Saved {}.",
                                                                    path.display()
                                                                )))),
                                                                Err(err) => notice.set(Some(Notice::error(format!(
                                                                    "Saved {}, but failed to apply proxy endpoints: {err}",
                                                                    path.display()
                                                                )))),
                                                            }
                                                        }
                                                        Err(err) => {
                                                            config.set(previous);
                                                            notice.set(Some(Notice::error(err)));
                                                        }
                                                    }
                                                }
                                                Err(err) => notice.set(Some(Notice::error(err))),
                                            }
                                        }
                                    },
                                    "Save Advanced Overrides"
                                }
                                button {
                                    class: "btn btn-outline",
                                    onclick: {
                                        let pn = project_name.clone();
                                        move |_| {
                                            let current = config();
                                            let next = current
                                                .projects
                                                .get(&pn)
                                                .map(|project_cfg| project_cfg.proxy_endpoints.clone())
                                                .unwrap_or_default();
                                            proxy_endpoints_form.set(next);
                                        }
                                    },
                                    "Reset Overrides"
                                }
                            }
                        }

                        div { class: "form-actions",
                            button {
                                class: "btn btn-primary",
                                disabled: !edit_dirty,
                                onclick: {
                                    let pn = project_name.clone();
                                    move |_| {
                                        let form = edit_form();
                                        let update_input = UpdateProjectInput {
                                            dir: form.dir,
                                            ip: form.ip,
                                            services: form.services,
                                        };
                                        let previous = config();
                                        let update_result = {
                                            let mut cfg = config.write();
                                            loopbox::update_project(&mut cfg, &pn, &update_input)
                                        };

                                        match update_result {
                                            Ok(()) => {
                                                if let Some(updated) = config().projects.get(&pn).cloned() {
                                                    edit_form.set(ProjectEditForm::from_project(&updated));
                                                }
                                                persist_config_and_apply(
                                                    config,
                                                    notice,
                                                    pending_auto_apply,
                                                    format!("Updated '{pn}'."),
                                                    Some(previous),
                                                );
                                            }
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                },
                                "Save Changes"
                            }
                            button {
                                class: "btn btn-outline",
                                onclick: {
                                    let original = project.clone();
                                    move |_| {
                                        edit_form.set(ProjectEditForm::from_project(&original));
                                    }
                                },
                                "Reset"
                            }
                        }
                    }

                    section { class: "panel",
                        div { class: "panel-header",
                            h2 { "Shell Exports" }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let v = env_preview.clone();
                                    move |_| {
                                        let _ = copy_to_clipboard(&v);
                                        notice.set(Some(Notice::success("Copied exports to clipboard.")));
                                    }
                                },
                                "Copy"
                            }
                        }
                        textarea {
                            class: "code-box",
                            readonly: true,
                            value: "{env_preview}",
                        }
                    }

                    section { class: "panel",
                        div { class: "panel-header",
                            h2 { "Quick Start" }
                            button {
                                class: "btn btn-sm btn-outline",
                                onclick: {
                                    let v = run_hint.clone();
                                    move |_| {
                                        let _ = copy_to_clipboard(&v);
                                        notice.set(Some(Notice::success("Copied quick start to clipboard.")));
                                    }
                                },
                                "Copy"
                            }
                        }
                        textarea {
                            class: "code-box code-box-sm",
                            readonly: true,
                            value: "{run_hint}",
                        }
                    }

                    div { class: "danger-zone",
                        p { class: "danger-zone-label", "Danger Zone" }
                        div { class: "danger-zone-row",
                            div { class: "danger-zone-info",
                                span { class: "danger-zone-action-title", "Remove Sandbox" }
                                span { class: "danger-zone-action-desc",
                                    "Permanently deletes this sandbox configuration. Running services will be stopped."
                                }
                            }
                            button {
                                class: "btn btn-sm btn-danger",
                                onclick: {
                                    let pn = project_name.clone();
                                    move |_| {
                                        let previous = config();
                                        let remove_result = {
                                            let mut cfg = config.write();
                                            loopbox::remove_project(&mut cfg, &pn)
                                        };

                                        match remove_result {
                                            Ok(()) => {
                                                selected_project.set(None);
                                                persist_config_and_apply(
                                                    config,
                                                    notice,
                                                    pending_auto_apply,
                                                    format!("Removed '{pn}'."),
                                                    Some(previous),
                                                );
                                            }
                                            Err(err) => notice.set(Some(Notice::error(err))),
                                        }
                                    }
                                },
                                "Remove"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn decode_service_input_sequence(raw: &str) -> Result<String, String> {
    let mut decoded = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        let Some(escape) = chars.next() else {
            return Err("Invalid key sequence: trailing backslash.".to_string());
        };
        match escape {
            '\\' => decoded.push('\\'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            'x' => {
                let hi = chars.next().ok_or_else(|| {
                    "Invalid key sequence: expected two hex digits after \\x.".to_string()
                })?;
                let lo = chars.next().ok_or_else(|| {
                    "Invalid key sequence: expected two hex digits after \\x.".to_string()
                })?;
                let value = u8::from_str_radix(&format!("{hi}{lo}"), 16).map_err(|_| {
                    "Invalid key sequence: expected hex digits after \\x.".to_string()
                })?;
                decoded.push(char::from(value));
            }
            _ => {
                return Err(format!(
                    "Invalid key sequence: unsupported escape '\\{escape}'. Use \\\\, \\n, \\r, \\t, \\0, or \\xNN."
                ));
            }
        }
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(name: &str) -> ProxyEndpointConfig {
        ProxyEndpointConfig {
            name: name.to_string(),
            listen_host: "127.0.0.1".to_string(),
            listen_port: 50051,
            protocol: ProxyEndpointProtocol::GrpcH2c,
            upstream_host: "127.0.0.30".to_string(),
            upstream_port: 50051,
            authority: None,
            project_name: Some("demo".to_string()),
            service_name: None,
        }
    }

    #[test]
    fn decode_service_input_sequence_supports_common_escapes() {
        let decoded = decode_service_input_sequence("r\\n\\x12").expect("decode");
        assert_eq!(decoded.as_bytes(), b"r\n\x12");
    }

    #[test]
    fn decode_service_input_sequence_rejects_invalid_escape() {
        let err = decode_service_input_sequence("\\q").unwrap_err();
        assert!(err.contains("unsupported escape"));
    }

    #[test]
    fn project_proxy_endpoint_protocol_helpers_work() {
        assert_eq!(
            parse_project_proxy_endpoint_protocol("grpc_h2c"),
            Some(ProxyEndpointProtocol::GrpcH2c)
        );
        assert_eq!(
            parse_project_proxy_endpoint_protocol("http1"),
            Some(ProxyEndpointProtocol::Http1)
        );
        assert_eq!(
            parse_project_proxy_endpoint_protocol("tcp_passthrough"),
            Some(ProxyEndpointProtocol::TcpPassthrough)
        );
        assert_eq!(parse_project_proxy_endpoint_protocol("unknown"), None);
        assert_eq!(
            project_proxy_endpoint_protocol_value(&ProxyEndpointProtocol::GrpcH2c),
            "grpc_h2c"
        );
    }

    #[test]
    fn service_protocol_helpers_work() {
        assert_eq!(
            parse_service_protocol("grpc_h2c"),
            Some(ProxyEndpointProtocol::GrpcH2c)
        );
        assert_eq!(
            parse_service_protocol("http1"),
            Some(ProxyEndpointProtocol::Http1)
        );
        assert_eq!(
            parse_service_protocol("tcp_passthrough"),
            Some(ProxyEndpointProtocol::TcpPassthrough)
        );
        assert_eq!(parse_service_protocol("unknown"), None);
        assert_eq!(
            service_protocol_value(&ProxyEndpointProtocol::GrpcH2c),
            "grpc_h2c"
        );
    }

    #[test]
    fn normalize_grpc_proto_paths_for_form_dedupes_and_trims() {
        let parsed = normalize_grpc_proto_paths_for_form(
            " /tmp/proto \n\n/tmp/proto;/opt/api/proto, /opt/api/proto ",
        );
        assert_eq!(parsed, vec!["/tmp/proto", "/opt/api/proto"]);
    }

    #[test]
    fn sanitize_project_proxy_endpoints_sets_project_and_normalizes() {
        let mut endpoints = vec![ProxyEndpointConfig {
            name: " ".to_string(),
            listen_host: " ".to_string(),
            listen_port: 50051,
            protocol: ProxyEndpointProtocol::GrpcH2c,
            upstream_host: " 127.0.0.30 ".to_string(),
            upstream_port: 50051,
            authority: Some(" Gateway.Skybrid.Localhost ".to_string()),
            project_name: None,
            service_name: Some(" Gateway ".to_string()),
        }];

        sanitize_project_proxy_endpoints_for_form("Skybrid", &mut endpoints);
        let endpoint = &endpoints[0];
        assert_eq!(endpoint.name, "endpoint-1");
        assert_eq!(endpoint.listen_host, "127.0.0.1");
        assert_eq!(endpoint.upstream_host, "127.0.0.30");
        assert_eq!(endpoint.project_name.as_deref(), Some("skybrid"));
        assert_eq!(endpoint.service_name.as_deref(), Some("gateway"));
        assert_eq!(
            endpoint.authority.as_deref(),
            Some("gateway.skybrid.localhost")
        );
    }

    #[test]
    fn validate_project_proxy_endpoints_allows_distinct_grpc_authorities() {
        let mut endpoints = vec![endpoint("grpc-a"), endpoint("grpc-b")];
        endpoints[0].authority = Some("gateway.skybrid.localhost".to_string());
        endpoints[1].authority = Some("gateway.other.localhost".to_string());

        assert!(validate_project_proxy_endpoints_for_form(&endpoints).is_ok());
    }

    #[test]
    fn validate_project_proxy_endpoints_rejects_duplicate_grpc_authority_listener() {
        let mut endpoints = vec![endpoint("grpc-a"), endpoint("grpc-b")];
        endpoints[0].authority = Some("gateway.skybrid.localhost".to_string());
        endpoints[1].authority = Some("gateway.skybrid.localhost".to_string());

        let err = validate_project_proxy_endpoints_for_form(&endpoints).unwrap_err();
        assert!(err.contains("duplicates gRPC authority route"));
    }

    #[test]
    fn validate_project_proxy_endpoints_rejects_mixed_protocol_on_same_listener() {
        let mut endpoints = vec![endpoint("grpc"), endpoint("http")];
        endpoints[1].protocol = ProxyEndpointProtocol::Http1;

        let err = validate_project_proxy_endpoints_for_form(&endpoints).unwrap_err();
        assert!(err.contains("all routes on one listener must use the same protocol"));
    }

    #[test]
    fn validate_project_proxy_endpoints_rejects_non_grpc_authority_and_duplicate_listener() {
        let mut endpoints = vec![endpoint("http-a"), endpoint("http-b")];
        endpoints[0].protocol = ProxyEndpointProtocol::Http1;
        endpoints[0].authority = Some("bad".to_string());
        endpoints[1].protocol = ProxyEndpointProtocol::Http1;

        let err = validate_project_proxy_endpoints_for_form(&endpoints).unwrap_err();
        assert!(err.contains("authority is only valid for grpc_h2c"));

        endpoints[0].authority = None;
        let err = validate_project_proxy_endpoints_for_form(&endpoints).unwrap_err();
        assert!(err.contains("only grpc_h2c supports multiple routes per listener"));
    }

    #[test]
    fn service_entry_port_rows_fall_back_to_legacy_fields() {
        let entry = ServiceEntry {
            name: "api".to_string(),
            ports: vec![],
            port: "8080".to_string(),
            protocol: "http1".to_string(),
            runtime: "process".to_string(),
            command: "pnpm dev".to_string(),
            workdir: "/tmp".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: "/health".to_string(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        };
        let rows = service_entry_port_rows(&entry);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].port, "8080");
        assert_eq!(rows[0].protocol, "http1");
        assert_eq!(rows[0].health_path, "/health");
    }

    #[test]
    fn sync_service_entry_primary_port_updates_legacy_fields() {
        let mut entry = ServiceEntry {
            name: "gateway".to_string(),
            ports: vec![
                ServicePortEntry {
                    port: "50051".to_string(),
                    protocol: "grpc_h2c".to_string(),
                    health_path: String::new(),
                },
                ServicePortEntry {
                    port: "8080".to_string(),
                    protocol: "http1".to_string(),
                    health_path: "/health".to_string(),
                },
            ],
            port: String::new(),
            protocol: String::new(),
            runtime: "process".to_string(),
            command: "pnpm dev".to_string(),
            workdir: "/tmp".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        };

        sync_service_entry_primary_port(&mut entry);
        assert_eq!(entry.port, "50051");
        assert_eq!(entry.protocol, "grpc_h2c");
        assert_eq!(entry.health_path, "");
    }

    #[test]
    fn service_entry_configured_ports_deduplicates_and_ignores_invalid() {
        let entry = ServiceEntry {
            name: "api".to_string(),
            ports: vec![
                ServicePortEntry {
                    port: "8080".to_string(),
                    protocol: "http1".to_string(),
                    health_path: String::new(),
                },
                ServicePortEntry {
                    port: "abc".to_string(),
                    protocol: "http1".to_string(),
                    health_path: String::new(),
                },
                ServicePortEntry {
                    port: "8080".to_string(),
                    protocol: "http1".to_string(),
                    health_path: String::new(),
                },
                ServicePortEntry {
                    port: "50051".to_string(),
                    protocol: "grpc_h2c".to_string(),
                    health_path: String::new(),
                },
            ],
            port: String::new(),
            protocol: "http1".to_string(),
            runtime: "process".to_string(),
            command: "pnpm dev".to_string(),
            workdir: "/tmp".to_string(),
            env_files: String::new(),
            depends_on: String::new(),
            autostart: false,
            health_path: String::new(),
            container_image: String::new(),
            container_args: String::new(),
            container_env: String::new(),
            container_volumes: String::new(),
            container_auto_remove: false,
        };

        assert_eq!(service_entry_configured_ports(&entry), vec![8080, 50051]);
    }
}
