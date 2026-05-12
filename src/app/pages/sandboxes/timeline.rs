use super::*;

const TIMELINE_EVENT_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimelineSeverityFilter {
    All,
    WarningCritical,
    Critical,
}

impl TimelineSeverityFilter {
    fn options() -> [Self; 3] {
        [Self::All, Self::WarningCritical, Self::Critical]
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::WarningCritical => "warning+",
            Self::Critical => "critical",
        }
    }
}

#[component]
pub(super) fn ProjectDetailTimelineTab(
    project_name: String,
    project: ProjectConfig,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
    mut current_page: Signal<Page>,
) -> Element {
    let mut service_filter = use_signal(|| None::<String>);
    let mut window_filter = use_signal(|| "1h".to_string());
    let mut severity_filter = use_signal(|| TimelineSeverityFilter::WarningCritical);
    let mut selected_event_id = use_signal(|| None::<String>);

    let timeline_project = project_name.clone();
    let timeline_events = use_resource(move || {
        let _tick = runtime_tick();
        let cfg = config();
        let selected_service = service_filter();
        let selected_window = window_filter();
        let timeline_project = timeline_project.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                loopbox::incident_timeline_for_project(
                    &cfg,
                    &timeline_project,
                    selected_service.as_deref(),
                    &selected_window,
                    TIMELINE_EVENT_LIMIT,
                )
            })
            .await
            .unwrap_or_else(|err| Err(format!("Incident timeline task failed: {err}")))
        }
    });

    let events_result = timeline_events();
    let timeline_loading = events_result.is_none();
    let (all_events, timeline_error) = match events_result {
        Some(Ok(events)) => (events, None),
        Some(Err(err)) => (Vec::new(), Some(err)),
        None => (Vec::new(), None),
    };
    let service_filter_snapshot = service_filter();
    let window_filter_snapshot = window_filter();
    let severity_filter_snapshot = severity_filter();
    let visible_events = all_events
        .iter()
        .filter(|event| timeline_event_matches_filter(event, severity_filter_snapshot))
        .cloned()
        .collect::<Vec<_>>();
    let selected_event = selected_event_id()
        .and_then(|selected_id| {
            visible_events
                .iter()
                .find(|event| event.id == selected_id)
                .cloned()
        })
        .or_else(|| visible_events.first().cloned());
    let selected_event_id_snapshot = selected_event.as_ref().map(|event| event.id.clone());
    let state_message = timeline_state_message(
        timeline_loading,
        timeline_error.as_deref(),
        all_events.len(),
        visible_events.len(),
    );
    let summary_label = if timeline_loading {
        "loading".to_string()
    } else if let Some(error) = timeline_error.as_ref() {
        format!("error: {error}")
    } else {
        format!(
            "{} visible / {} total",
            visible_events.len(),
            all_events.len()
        )
    };

    rsx! {
        div { class: "tab-content-timeline",
            div { class: "timeline-toolbar",
                div { class: "timeline-toolbar-row",
                    div { class: "timeline-filter-group timeline-filter-services",
                        span { class: "timeline-filter-label", "service" }
                        button {
                            class: if service_filter_snapshot.is_none() { "btn btn-sm btn-toggle-on" } else { "btn btn-sm btn-outline" },
                            onclick: move |_| {
                                service_filter.set(None);
                                selected_event_id.set(None);
                            },
                            "all"
                        }
                        for service in &project.services {
                            button {
                                key: "timeline-service-{service.name}",
                                class: if service_filter_snapshot.as_ref() == Some(&service.name) {
                                    "btn btn-sm btn-toggle-on"
                                } else {
                                    "btn btn-sm btn-outline"
                                },
                                onclick: {
                                    let service_name = service.name.clone();
                                    move |_| {
                                        service_filter.set(Some(service_name.clone()));
                                        selected_event_id.set(None);
                                    }
                                },
                                "{service.name}"
                            }
                        }
                    }
                    div { class: "timeline-filter-group",
                        span { class: "timeline-filter-label", "window" }
                        div { class: "seg-control",
                            for window in ["15m", "1h", "24h", "7d"] {
                                button {
                                    key: "timeline-window-{window}",
                                    class: if window_filter_snapshot == window { "seg-btn seg-btn-on" } else { "seg-btn" },
                                    onclick: move |_| {
                                        window_filter.set(window.to_string());
                                        selected_event_id.set(None);
                                    },
                                    "{window}"
                                }
                            }
                        }
                    }
                    div { class: "timeline-filter-group",
                        span { class: "timeline-filter-label", "severity" }
                        div { class: "timeline-filter-pills",
                            for filter in TimelineSeverityFilter::options() {
                                button {
                                    key: "timeline-severity-{filter.label()}",
                                    class: if severity_filter_snapshot == filter {
                                        "timeline-filter-pill timeline-filter-pill-on"
                                    } else {
                                        "timeline-filter-pill"
                                    },
                                    onclick: move |_| {
                                        severity_filter.set(filter);
                                        selected_event_id.set(None);
                                    },
                                    "{filter.label()}"
                                }
                            }
                        }
                    }
                }
                div { class: "timeline-toolbar-row timeline-toolbar-row-compact",
                    span { class: "timeline-summary-chip", "{summary_label}" }
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: move |_| runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1)),
                        "Refresh"
                    }
                }
            }

            div { class: "timeline-split",
                div { class: "timeline-list-pane",
                    if let Some(message) = &state_message {
                        div { class: if timeline_error.is_some() { "timeline-empty timeline-empty-error" } else { "timeline-empty" },
                            p { "{message}" }
                        }
                    } else {
                        div { class: "timeline-list-header",
                            span { class: "timeline-list-count", "{visible_events.len()} incidents" }
                            span { class: "timeline-list-window", "{window_filter_snapshot}" }
                        }
                        for event in &visible_events {
                            {{
                                let severity_class = timeline_severity_class(event.severity);
                                let severity_label = timeline_severity_label(event.severity);
                                let kind_label = timeline_kind_label(event.kind);
                                let service_label = timeline_service_label(event);
                                let selected = selected_event_id_snapshot.as_ref() == Some(&event.id);
                                rsx! {
                                    button {
                                        key: "timeline-event-{event.id}",
                                        class: if selected {
                                            "timeline-row timeline-row-selected"
                                        } else {
                                            "timeline-row"
                                        },
                                        onclick: {
                                            let event_id = event.id.clone();
                                            move |_| selected_event_id.set(Some(event_id.clone()))
                                        },
                                        span { class: "timeline-severity-dot {severity_class}" }
                                        div { class: "timeline-row-main",
                                            span { class: "timeline-row-summary", "{event.summary}" }
                                            span { class: "timeline-row-meta",
                                                "{kind_label} · {service_label} · {event.occurred_at_utc}"
                                            }
                                        }
                                        span { class: "timeline-row-severity {severity_class}", "{severity_label}" }
                                    }
                                }
                            }}
                        }
                    }
                }

                div { class: "timeline-detail-pane",
                    if let Some(event) = selected_event {
                        {{
                            let severity_class = timeline_severity_class(event.severity);
                            let severity_label = timeline_severity_label(event.severity);
                            let kind_label = timeline_kind_label(event.kind);
                            let service_label = timeline_service_label(&event);
                            rsx! {
                                div { class: "timeline-detail-header",
                                    span { class: "timeline-detail-severity {severity_class}", "{severity_label}" }
                                    span { class: "timeline-detail-title", "{event.summary}" }
                                    div { class: "timeline-detail-actions",
                                        button {
                                            class: "traffic-copy-req-btn",
                                            title: "Copy incident evidence",
                                            onclick: {
                                                let text = timeline_event_clipboard_text(&event);
                                                move |_| {
                                                    match copy_to_clipboard(&text) {
                                                        Ok(()) => notice.set(Some(Notice::success("Copied incident evidence."))),
                                                        Err(err) => notice.set(Some(Notice::error(err))),
                                                    }
                                                }
                                            },
                                            "copy evidence"
                                        }
                                        button {
                                            class: "traffic-copy-req-btn",
                                            title: "Ask Codex to diagnose this incident",
                                            onclick: {
                                                let event_id = event.id.clone();
                                                let project_name = project_name.clone();
                                                let service_name = service_filter_snapshot
                                                    .clone()
                                                    .or_else(|| event.service_name.clone());
                                                let window = window_filter_snapshot.clone();
                                                move |_| {
                                                    match loopbox::create_diagnosis_session(
                                                        &config(),
                                                        loopbox::CreateDiagnosisSessionInput {
                                                            project_name: project_name.clone(),
                                                            service_name: service_name.clone(),
                                                            source: loopbox::DiagnosisSource::Incident,
                                                            window: window.clone(),
                                                            incident_id: Some(event_id.clone()),
                                                            title: None,
                                                        },
                                                    ) {
                                                        Ok(session) => {
                                                            let prompt = loopbox::diagnosis_prompt_for_session(&session);
                                                            loopbox::codex_agents_prefill_diagnosis_prompt(session.id, prompt);
                                                            current_page.set(Page::Agents);
                                                        }
                                                        Err(err) => notice.set(Some(Notice::error(err))),
                                                    }
                                                }
                                            },
                                            "Ask Agent"
                                        }
                                    }
                                }
                                div { class: "traffic-detail-meta timeline-detail-meta",
                                    div { class: "traffic-detail-field",
                                        span { class: "traffic-detail-label", "time" }
                                        span { class: "traffic-detail-value", "{event.occurred_at_utc}" }
                                    }
                                    div { class: "traffic-detail-field",
                                        span { class: "traffic-detail-label", "service" }
                                        span { class: "traffic-detail-value", "{service_label}" }
                                    }
                                    div { class: "traffic-detail-field",
                                        span { class: "traffic-detail-label", "kind" }
                                        span { class: "traffic-detail-value", "{kind_label}" }
                                    }
                                    div { class: "traffic-detail-field",
                                        span { class: "traffic-detail-label", "source" }
                                        span { class: "traffic-detail-value", "{event.source}" }
                                    }
                                }
                                if let Some(detail) = &event.detail {
                                    div { class: "timeline-detail-note",
                                        "{detail}"
                                    }
                                }
                                div { class: "traffic-detail-section",
                                    div { class: "traffic-section-title",
                                        span { "Evidence" }
                                        span { class: "timeline-evidence-count", "{event.evidence.len()} item(s)" }
                                    }
                                    if event.evidence.is_empty() {
                                        div { class: "timeline-evidence-empty", "No attached evidence." }
                                    } else {
                                        div { class: "timeline-evidence-list",
                                            for (index, evidence) in event.evidence.iter().enumerate() {
                                                {{
                                                    let evidence_label = timeline_evidence_label(evidence);
                                                    let evidence_text = timeline_evidence_text(evidence);
                                                    rsx! {
                                                        div { class: "timeline-evidence-card", key: "timeline-evidence-{event.id}-{index}",
                                                            div { class: "timeline-evidence-title", "{evidence_label}" }
                                                            pre { class: "timeline-evidence-body", "{evidence_text}" }
                                                        }
                                                    }
                                                }}
                                            }
                                        }
                                    }
                                }
                            }
                        }}
                    } else {
                        div { class: "traffic-detail-empty", "Select an incident to inspect evidence." }
                    }
                }
            }
        }
    }
}

pub(super) fn timeline_event_matches_filter(
    event: &IncidentTimelineEvent,
    filter: TimelineSeverityFilter,
) -> bool {
    match filter {
        TimelineSeverityFilter::All => true,
        TimelineSeverityFilter::WarningCritical => {
            matches!(
                event.severity,
                IncidentSeverity::Warning | IncidentSeverity::Critical
            )
        }
        TimelineSeverityFilter::Critical => event.severity == IncidentSeverity::Critical,
    }
}

pub(super) fn timeline_state_message(
    loading: bool,
    error: Option<&str>,
    total_events: usize,
    visible_events: usize,
) -> Option<String> {
    if loading {
        return Some("Loading incident timeline...".to_string());
    }
    if let Some(error) = error {
        return Some(error.to_string());
    }
    if total_events == 0 {
        return Some("No incidents found for this window.".to_string());
    }
    if visible_events == 0 {
        return Some("No incidents match the active severity filter.".to_string());
    }
    None
}

pub(super) fn timeline_severity_label(severity: IncidentSeverity) -> &'static str {
    match severity {
        IncidentSeverity::Info => "info",
        IncidentSeverity::Warning => "warning",
        IncidentSeverity::Critical => "critical",
    }
}

pub(super) fn timeline_severity_class(severity: IncidentSeverity) -> &'static str {
    match severity {
        IncidentSeverity::Info => "timeline-severity-info",
        IncidentSeverity::Warning => "timeline-severity-warning",
        IncidentSeverity::Critical => "timeline-severity-critical",
    }
}

pub(super) fn timeline_kind_label(kind: IncidentKind) -> &'static str {
    match kind {
        IncidentKind::RuntimeTransition => "runtime",
        IncidentKind::TrafficFailure => "traffic failure",
        IncidentKind::SlowRequest => "slow request",
        IncidentKind::ResourcePressure => "resource pressure",
        IncidentKind::ResourceUnavailable => "resource unavailable",
    }
}

fn timeline_service_label(event: &IncidentTimelineEvent) -> String {
    event
        .service_name
        .clone()
        .unwrap_or_else(|| "project".to_string())
}

pub(super) fn timeline_evidence_label(evidence: &IncidentEvidence) -> &'static str {
    match evidence {
        IncidentEvidence::RuntimeSnapshot { .. } => "runtime snapshot",
        IncidentEvidence::RequestSummary { .. } => "request summary",
        IncidentEvidence::ResourceSampleSummary { .. } => "resource sample",
        IncidentEvidence::LogExcerpt { .. } => "log excerpt",
    }
}

pub(super) fn timeline_evidence_text(evidence: &IncidentEvidence) -> String {
    match evidence {
        IncidentEvidence::RuntimeSnapshot {
            state,
            pid,
            started_at,
            exit_code,
            last_error,
        } => format!(
            "state: {}\npid: {}\nstarted_at: {}\nexit_code: {}\nlast_error: {}",
            service_runtime_state_label(*state),
            optional_display(pid.map(|value| value.to_string())),
            optional_display(started_at.map(|value| value.to_string())),
            optional_display(exit_code.map(|value| value.to_string())),
            optional_display(last_error.clone()),
        ),
        IncidentEvidence::RequestSummary {
            method,
            path,
            status_code,
            duration_ms,
            error,
        } => format!(
            "method: {method}\npath: {path}\nstatus: {}\nduration: {duration_ms}ms\nerror: {}",
            optional_display(status_code.map(|value| value.to_string())),
            optional_display(error.clone()),
        ),
        IncidentEvidence::ResourceSampleSummary {
            sampled_at_utc,
            cpu_percent,
            memory_bytes,
            process_count,
            unavailable_reason,
        } => format!(
            "sampled_at: {sampled_at_utc}\ncpu: {}\nmemory: {}\nprocesses: {}\nunavailable: {}",
            format_cpu_percent(*cpu_percent),
            format_memory_bytes(*memory_bytes),
            optional_display(process_count.map(|value| value.to_string())),
            optional_display(unavailable_reason.clone()),
        ),
        IncidentEvidence::LogExcerpt { service_name, line } => {
            format!("[{service_name}] {line}")
        }
    }
}

pub(super) fn timeline_event_clipboard_text(event: &IncidentTimelineEvent) -> String {
    let mut out = String::new();
    out.push_str(&format!("Incident: {}\n", event.summary));
    out.push_str(&format!("Project: {}\n", event.project_name));
    out.push_str(&format!("Service: {}\n", timeline_service_label(event)));
    out.push_str(&format!(
        "Severity: {}\n",
        timeline_severity_label(event.severity)
    ));
    out.push_str(&format!("Kind: {}\n", timeline_kind_label(event.kind)));
    out.push_str(&format!("Occurred: {}\n", event.occurred_at_utc));
    out.push_str(&format!("Source: {}\n", event.source));
    if let Some(detail) = event.detail.as_ref() {
        out.push_str(&format!("Detail: {detail}\n"));
    }
    if !event.evidence.is_empty() {
        out.push_str("\nEvidence:\n");
        for evidence in &event.evidence {
            out.push_str(&format!(
                "\n[{}]\n{}\n",
                timeline_evidence_label(evidence),
                timeline_evidence_text(evidence)
            ));
        }
    }
    out
}

fn optional_display(value: Option<String>) -> String {
    value.unwrap_or_else(|| "n/a".to_string())
}

fn service_runtime_state_label(state: ServiceRuntimeState) -> &'static str {
    match state {
        ServiceRuntimeState::Stopped => "stopped",
        ServiceRuntimeState::Starting => "starting",
        ServiceRuntimeState::Running => "running",
        ServiceRuntimeState::Unhealthy => "unhealthy",
        ServiceRuntimeState::Crashed => "crashed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(severity: IncidentSeverity) -> IncidentTimelineEvent {
        IncidentTimelineEvent {
            id: "incident-1".to_string(),
            occurred_at_unix_ms: 1,
            occurred_at_utc: "2026-05-05 12:00:00 UTC".to_string(),
            project_name: "demo".to_string(),
            service_name: Some("web".to_string()),
            severity,
            kind: IncidentKind::TrafficFailure,
            summary: "GET /api returned 503".to_string(),
            detail: Some("upstream failed".to_string()),
            evidence: vec![IncidentEvidence::RequestSummary {
                method: "GET".to_string(),
                path: "/api".to_string(),
                status_code: Some(503),
                duration_ms: 122,
                error: None,
            }],
            source: "traffic".to_string(),
        }
    }

    #[test]
    fn timeline_severity_labels_and_classes_are_stable() {
        assert_eq!(timeline_severity_label(IncidentSeverity::Info), "info");
        assert_eq!(
            timeline_severity_class(IncidentSeverity::Warning),
            "timeline-severity-warning"
        );
        assert_eq!(
            timeline_severity_class(IncidentSeverity::Critical),
            "timeline-severity-critical"
        );
    }

    #[test]
    fn timeline_filter_logic_matches_severity_thresholds() {
        let info = event(IncidentSeverity::Info);
        let warning = event(IncidentSeverity::Warning);
        let critical = event(IncidentSeverity::Critical);

        assert!(timeline_event_matches_filter(
            &info,
            TimelineSeverityFilter::All
        ));
        assert!(!timeline_event_matches_filter(
            &info,
            TimelineSeverityFilter::WarningCritical
        ));
        assert!(timeline_event_matches_filter(
            &warning,
            TimelineSeverityFilter::WarningCritical
        ));
        assert!(timeline_event_matches_filter(
            &critical,
            TimelineSeverityFilter::Critical
        ));
    }

    #[test]
    fn timeline_state_messages_cover_loading_empty_and_filtered_states() {
        assert_eq!(
            timeline_state_message(true, None, 0, 0).as_deref(),
            Some("Loading incident timeline...")
        );
        assert_eq!(
            timeline_state_message(false, Some("bad window"), 0, 0).as_deref(),
            Some("bad window")
        );
        assert_eq!(
            timeline_state_message(false, None, 0, 0).as_deref(),
            Some("No incidents found for this window.")
        );
        assert_eq!(
            timeline_state_message(false, None, 2, 0).as_deref(),
            Some("No incidents match the active severity filter.")
        );
        assert!(timeline_state_message(false, None, 2, 1).is_none());
    }

    #[test]
    fn timeline_evidence_rendering_is_copyable_text() {
        let event = event(IncidentSeverity::Critical);
        let evidence = &event.evidence[0];

        assert_eq!(timeline_evidence_label(evidence), "request summary");
        assert!(timeline_evidence_text(evidence).contains("status: 503"));
        assert!(timeline_event_clipboard_text(&event).contains("GET /api returned 503"));
        assert!(timeline_event_clipboard_text(&event).contains("[request summary]"));
    }
}
