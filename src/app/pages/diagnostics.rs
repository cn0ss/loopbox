use crate::app::models::{Notice, Page};
use crate::app::utils::copy_to_clipboard;
use crate::loopbox::{self, DiagnosisSession, DiagnosisStatus, LoopboxConfig};
use dioxus::prelude::*;
use std::collections::BTreeMap;

pub(in crate::app) fn render_diagnostics_page(
    page: Page,
    current_page: Signal<Page>,
    config: Signal<LoopboxConfig>,
    notice: Signal<Option<Notice>>,
    runtime_tick: Signal<u64>,
) -> Element {
    let mut selected_session_id = use_signal(|| None::<String>);
    let resolution_notes = use_signal(BTreeMap::<String, String>::new);
    let sessions_resource = use_resource(move || {
        let active = page == Page::Diagnostics;
        let tick = if active { Some(runtime_tick()) } else { None };
        async move {
            if !active {
                return Vec::new();
            }
            let _ = tick;
            tokio::task::spawn_blocking(|| loopbox::diagnosis_sessions(200).unwrap_or_default())
                .await
                .unwrap_or_default()
        }
    });

    if page != Page::Diagnostics {
        return rsx! {};
    }

    let sessions = sessions_resource()
        .unwrap_or_default()
        .into_iter()
        .filter(|session| session.status != DiagnosisStatus::Archived)
        .collect::<Vec<_>>();
    let active_count = sessions
        .iter()
        .filter(|session| {
            matches!(
                session.status,
                DiagnosisStatus::Draft | DiagnosisStatus::InProgress | DiagnosisStatus::Completed
            )
        })
        .count();
    let selected_id = selected_session_id();
    let selected_session = selected_id
        .as_ref()
        .and_then(|id| sessions.iter().find(|session| &session.id == id))
        .or_else(|| sessions.first());

    rsx! {
        div { class: "page diagnostics-page",
            div { class: "page-header",
                div { class: "page-header-left",
                    div { class: "page-header-stack",
                        span { class: "page-eyebrow", "Investigate" }
                        div {
                            style: "display:flex; align-items:baseline; gap:14px; flex-wrap:wrap;",
                            h1 { class: "page-title", "diagnostics" }
                            span { class: "status-badge status-badge--neutral", "{active_count} active" }
                        }
                        p { class: "page-subtitle",
                            "Bundled logs, runtime, and traffic for an incident — handed to an agent for root-cause analysis."
                        }
                    }
                }
            }

            if sessions.is_empty() {
                div { class: "empty-state diagnostics-empty",
                    div { class: "empty-state-icon", "—" }
                    h2 { class: "empty-state-title", "no diagnosis sessions" }
                    p { class: "empty-state-desc",
                        "Diagnoses bundle the relevant logs, runtime, and traffic for an incident so an agent can reason over it. Open one from a sandbox, runtime alert, or the incident timeline."
                    }
                }
            } else {
                div { class: "diagnostics-layout",
                    section { class: "diagnostics-list",
                        div { class: "diagnostics-list-head",
                            span { "Sessions" }
                            span { "{sessions.len()}" }
                        }
                        for session in sessions.iter() {
                            {{
                                let selected = selected_session
                                    .map(|selected| selected.id == session.id)
                                    .unwrap_or(false);
                                let session_id = session.id.clone();
                                rsx! {
                                    button {
                                        key: "{session.id}",
                                        class: if selected { "diagnosis-row is-selected" } else { "diagnosis-row" },
                                        onclick: move |_| selected_session_id.set(Some(session_id.clone())),
                                        span { class: "diagnosis-row-top",
                                            strong { "{session.title}" }
                                            if session.report.is_some() {
                                                span { class: "diagnosis-report-badge", "report" }
                                            }
                                            span { class: format!("diagnosis-status {}", diagnosis_status_class(session.status)), "{session.status.label()}" }
                                        }
                                        span { class: "diagnosis-row-meta",
                                            "{session.project_name} · {diagnosis_service_label(session)} · {session.created_at_utc}"
                                        }
                                        span { class: "diagnosis-row-evidence",
                                            "{session.source.label()} · {session.evidence.evidence_count()} evidence item(s)"
                                        }
                                        if let Some(report) = session.report.as_ref() {
                                            span { class: "diagnosis-row-report-summary", "{report.summary}" }
                                        }
                                    }
                                }
                            }}
                        }
                    }

                    section { class: "diagnostics-detail",
                        if let Some(session) = selected_session {
                            {render_diagnosis_detail(
                                session,
                                current_page,
                                config,
                                notice,
                                runtime_tick,
                                resolution_notes,
                            )}
                        } else {
                            div { class: "traffic-detail-empty", "Select a diagnosis session." }
                        }
                    }
                }
            }
        }
    }
}

fn render_diagnosis_detail(
    session: &DiagnosisSession,
    mut current_page: Signal<Page>,
    config: Signal<LoopboxConfig>,
    mut notice: Signal<Option<Notice>>,
    mut runtime_tick: Signal<u64>,
    mut resolution_notes: Signal<BTreeMap<String, String>>,
) -> Element {
    let evidence_text = diagnosis_evidence_text(session);
    let report_text = diagnosis_report_copy_text(session);
    let session_id_for_agent = session.id.clone();
    let prompt = loopbox::diagnosis_prompt_for_session(session);
    let thread_id = session.linked_thread_id.clone();
    let resolve_id = session.id.clone();
    let archive_id = session.id.clone();
    let resolution_note_value = resolution_notes
        .with(|notes| notes.get(&session.id).cloned())
        .unwrap_or_else(|| session.resolution_note.clone().unwrap_or_default());
    let resolution_note_input_id = session.id.clone();
    let existing_resolution_note = session.resolution_note.clone().unwrap_or_default();

    rsx! {
        div { class: "diagnosis-detail-header",
            div {
                span { class: format!("diagnosis-status {}", diagnosis_status_class(session.status)), "{session.status.label()}" }
                h2 { "{session.title}" }
                p { "{session.project_name} · {diagnosis_service_label(session)} · {session.window}" }
            }
            div { class: "diagnosis-detail-actions",
                if let Some(thread_id) = thread_id.clone() {
                    button {
                        class: "btn btn-sm btn-primary",
                        onclick: {
                            let thread_id = thread_id.clone();
                            move |_| match loopbox::codex_agents_resume_thread(&config(), &thread_id) {
                                Ok(()) => current_page.set(Page::Agents),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Open Agent Thread"
                    }
                } else {
                    button {
                        class: "btn btn-sm btn-primary",
                        onclick: move |_| {
                            loopbox::codex_agents_prefill_diagnosis_prompt(
                                session_id_for_agent.clone(),
                                prompt.clone(),
                            );
                            current_page.set(Page::Agents);
                        },
                        "Start Agent"
                    }
                }
                if session.report.is_some() {
                    button {
                        class: "btn btn-sm btn-outline",
                        onclick: {
                            let report_text = report_text.clone();
                            move |_| match copy_to_clipboard(&report_text) {
                                Ok(()) => notice.set(Some(Notice::success("Copied diagnosis report."))),
                                Err(err) => notice.set(Some(Notice::error(err))),
                            }
                        },
                        "Copy Report"
                    }
                }
                button {
                    class: "btn btn-sm btn-outline",
                    onclick: {
                        let evidence_text = evidence_text.clone();
                        move |_| match copy_to_clipboard(&evidence_text) {
                            Ok(()) => notice.set(Some(Notice::success("Copied diagnosis evidence."))),
                            Err(err) => notice.set(Some(Notice::error(err))),
                        }
                    },
                    "Copy Evidence"
                }
                button {
                    class: "btn btn-sm btn-outline",
                    disabled: session.status == DiagnosisStatus::Resolved,
                    onclick: move |_| {
                        let resolution_note = resolution_notes
                            .with(|notes| notes.get(&resolve_id).cloned())
                            .unwrap_or_else(|| existing_resolution_note.clone());
                        match loopbox::resolve_diagnosis_session(&resolve_id, &resolution_note) {
                            Ok(_) => {
                                runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                notice.set(Some(Notice::success("Diagnosis marked resolved.")));
                            }
                            Err(err) => notice.set(Some(Notice::error(err))),
                        }
                    },
                    "Mark Resolved"
                }
                button {
                    class: "btn btn-sm btn-outline",
                    onclick: move |_| {
                        match loopbox::update_diagnosis_session_status(&archive_id, DiagnosisStatus::Archived) {
                            Ok(_) => {
                                runtime_tick.with_mut(|tick| *tick = tick.wrapping_add(1));
                                notice.set(Some(Notice::info("Diagnosis archived.")));
                            }
                            Err(err) => notice.set(Some(Notice::error(err))),
                        }
                    },
                    "Archive"
                }
            }
        }

        div { class: "diagnosis-evidence-grid",
            DiagnosisEvidenceTile { label: "Incidents".to_string(), value: session.evidence.incidents.len().to_string() }
            DiagnosisEvidenceTile { label: "Runtime".to_string(), value: session.evidence.runtime.len().to_string() }
            DiagnosisEvidenceTile {
                label: "Logs".to_string(),
                value: session.evidence.log_tails.iter().map(|tail| tail.lines.len()).sum::<usize>().to_string(),
            }
            DiagnosisEvidenceTile { label: "Requests".to_string(), value: session.evidence.requests.len().to_string() }
            DiagnosisEvidenceTile { label: "Resources".to_string(), value: session.evidence.resources.len().to_string() }
            DiagnosisEvidenceTile { label: "Doctor".to_string(), value: session.evidence.doctor_issues.len().to_string() }
        }

        div { class: "diagnosis-detail-body",
            if let Some(report) = session.report.as_ref() {
                section { class: "diagnosis-section diagnosis-report-section",
                    div { class: "diagnosis-section-heading-row",
                        h3 { "Agent Report" }
                        span { class: "diagnosis-report-time", "{report.captured_at_utc}" }
                    }
                    div { class: "diagnosis-report-summary", "{report.summary}" }
                    pre { class: "diagnosis-report-body", "{report.agent_message}" }
                }
            }
            section { class: "diagnosis-section diagnosis-resolution-section",
                div { class: "diagnosis-section-heading-row",
                    h3 { "Resolution" }
                    if let Some(resolved_at_utc) = session.resolved_at_utc.as_ref() {
                        span { class: "diagnosis-report-time", "{resolved_at_utc}" }
                    }
                }
                textarea {
                    class: "diagnosis-resolution-note",
                    value: "{resolution_note_value}",
                    placeholder: "Record the fix or operational decision...",
                    oninput: move |evt| {
                        let value = evt.value();
                        resolution_notes.with_mut(|notes| {
                            notes.insert(resolution_note_input_id.clone(), value);
                        });
                    },
                }
            }
            if let Some(incident) = session.evidence.selected_incident.as_ref() {
                section { class: "diagnosis-section",
                    h3 { "Selected Incident" }
                    p { "{incident.summary}" }
                    code { "{incident.id}" }
                }
            }
            section { class: "diagnosis-section",
                h3 { "Evidence Snapshot" }
                pre { "{evidence_text}" }
            }
        }
    }
}

#[component]
fn DiagnosisEvidenceTile(label: String, value: String) -> Element {
    rsx! {
        div { class: "diagnosis-evidence-tile",
            span { "{label}" }
            strong { "{value}" }
        }
    }
}

fn diagnosis_status_class(status: DiagnosisStatus) -> &'static str {
    match status {
        DiagnosisStatus::Draft => "is-draft",
        DiagnosisStatus::InProgress => "is-progress",
        DiagnosisStatus::Completed => "is-completed",
        DiagnosisStatus::Resolved => "is-resolved",
        DiagnosisStatus::Archived => "is-archived",
    }
}

fn diagnosis_service_label(session: &DiagnosisSession) -> String {
    session
        .service_name
        .clone()
        .unwrap_or_else(|| "all services".to_string())
}

fn diagnosis_evidence_text(session: &DiagnosisSession) -> String {
    let mut out = String::new();
    out.push_str(&format!("Diagnosis: {}\n", session.id));
    out.push_str(&format!("Project: {}\n", session.project_name));
    out.push_str(&format!("Service: {}\n", diagnosis_service_label(session)));
    out.push_str(&format!("Status: {}\n", session.status.label()));
    out.push_str(&format!("Source: {}\n", session.source.label()));
    out.push_str(&format!("Window: {}\n", session.window));
    if let Some(thread_id) = session.linked_thread_id.as_ref() {
        out.push_str(&format!("Codex thread: {thread_id}\n"));
    }
    if let Some(incident) = session.evidence.selected_incident.as_ref() {
        out.push_str(&format!(
            "\nSelected incident:\n{} · {:?} · {}\n",
            incident.id, incident.severity, incident.summary
        ));
    }
    out.push_str("\nEvidence counts:\n");
    out.push_str(&format!(
        "- incidents: {}\n",
        session.evidence.incidents.len()
    ));
    out.push_str(&format!("- runtime: {}\n", session.evidence.runtime.len()));
    out.push_str(&format!(
        "- log lines: {}\n",
        session
            .evidence
            .log_tails
            .iter()
            .map(|tail| tail.lines.len())
            .sum::<usize>()
    ));
    out.push_str(&format!(
        "- requests: {}\n",
        session.evidence.requests.len()
    ));
    out.push_str(&format!(
        "- resources: {}\n",
        session.evidence.resources.len()
    ));
    out.push_str(&format!(
        "- doctor: {}\n",
        session.evidence.doctor_issues.len()
    ));

    if !session.evidence.doctor_issues.is_empty() {
        out.push_str("\nDoctor issues:\n");
        for issue in &session.evidence.doctor_issues {
            out.push_str(&format!("- {:?}: {}\n", issue.level, issue.message));
        }
    }
    if !session.evidence.log_tails.is_empty() {
        out.push_str("\nLog excerpts:\n");
        for tail in &session.evidence.log_tails {
            for line in tail.lines.iter().rev().take(8).rev() {
                out.push_str(&format!("[{}] {}\n", tail.service_name, line));
            }
        }
    }
    out
}

fn diagnosis_report_copy_text(session: &DiagnosisSession) -> String {
    let mut out = String::new();
    out.push_str(&format!("Diagnosis: {}\n", session.id));
    out.push_str(&format!("Project: {}\n", session.project_name));
    out.push_str(&format!("Service: {}\n", diagnosis_service_label(session)));
    out.push_str(&format!("Status: {}\n", session.status.label()));
    if let Some(thread_id) = session.linked_thread_id.as_ref() {
        out.push_str(&format!("Codex thread: {thread_id}\n"));
    }
    if let Some(report) = session.report.as_ref() {
        out.push_str(&format!("Report captured: {}\n", report.captured_at_utc));
        if let Some(turn_id) = report.turn_id.as_ref() {
            out.push_str(&format!("Codex turn: {turn_id}\n"));
        }
        out.push_str(&format!("Summary: {}\n", report.summary));
        out.push_str("\nAgent answer:\n");
        out.push_str(&report.agent_message);
        out.push('\n');
    } else {
        out.push_str("\nNo agent report captured.\n");
    }
    if let Some(note) = session
        .resolution_note
        .as_ref()
        .filter(|note| !note.trim().is_empty())
    {
        out.push_str("\nResolution note:\n");
        out.push_str(note);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_report() -> DiagnosisSession {
        DiagnosisSession {
            id: "diag-1".to_string(),
            created_at_unix_ms: 1_777_000_000_000,
            created_at_utc: "2026-05-06 12:00:00 UTC".to_string(),
            updated_at_unix_ms: 1_777_000_000_100,
            status: DiagnosisStatus::Completed,
            source: loopbox::DiagnosisSource::Incident,
            project_name: "demo".to_string(),
            service_name: Some("web".to_string()),
            window: "1h".to_string(),
            title: "GET /api returned 503".to_string(),
            linked_thread_id: Some("thread-1".to_string()),
            report: Some(loopbox::DiagnosisReport {
                captured_at_unix_ms: 1_777_000_000_200,
                captured_at_utc: "2026-05-06 12:00:02 UTC".to_string(),
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                summary: "Backend crashed after missing env.".to_string(),
                agent_message:
                    "Backend crashed after missing env.\nSet DATABASE_URL and restart web."
                        .to_string(),
            }),
            resolution_note: Some("Added DATABASE_URL.".to_string()),
            resolved_at_unix_ms: None,
            resolved_at_utc: None,
            evidence: loopbox::DiagnosisEvidenceSnapshot::default(),
        }
    }

    #[test]
    fn diagnosis_report_copy_text_includes_identity_summary_and_answer() {
        let text = diagnosis_report_copy_text(&session_with_report());

        assert!(text.contains("Diagnosis: diag-1"));
        assert!(text.contains("Project: demo"));
        assert!(text.contains("Summary: Backend crashed after missing env."));
        assert!(text.contains("Backend crashed after missing env."));
        assert!(text.contains("Set DATABASE_URL and restart web."));
    }
}
