#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![recursion_limit = "256"]

mod platform;
#[macro_use]
mod loopbox;
mod app;

fn suppress_noisy_h2_debug_logs() {
    let existing = std::env::var("RUST_LOG").ok();
    match existing {
        Some(filter) => {
            let has_h2_rule = filter
                .split(',')
                .map(str::trim)
                .any(|part| part.starts_with("h2=") || part == "h2");
            if !has_h2_rule {
                std::env::set_var("RUST_LOG", format!("{filter},h2=warn"));
            }
        }
        None => {
            std::env::set_var("RUST_LOG", "info,h2=warn");
        }
    }
}

fn desktop_lifecycle_mode() -> (dioxus::desktop::WindowCloseBehaviour, bool) {
    #[cfg(debug_assertions)]
    {
        (dioxus::desktop::WindowCloseBehaviour::WindowCloses, true)
    }

    #[cfg(not(debug_assertions))]
    {
        (dioxus::desktop::WindowCloseBehaviour::WindowHides, false)
    }
}

fn desktop_config() -> dioxus::desktop::Config {
    let (close_behaviour, exits_when_last_window_closes) = desktop_lifecycle_mode();

    dioxus::desktop::Config::new()
        .with_close_behaviour(close_behaviour)
        .with_exits_when_last_window_closes(exits_when_last_window_closes)
        .with_window(
            dioxus::desktop::WindowBuilder::new()
                .with_title("Loopbox")
                .with_inner_size(dioxus::desktop::LogicalSize::new(1280.0, 860.0))
                .with_min_inner_size(dioxus::desktop::LogicalSize::new(640.0, 480.0)),
        )
}

fn main() {
    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(exit_code) = loopbox::run_runtime_subcommand_from_args(&cli_args) {
        std::process::exit(exit_code);
    }
    if let Some(exit_code) = loopbox::run_agent_api_subcommand_from_args(&cli_args) {
        std::process::exit(exit_code);
    }
    if let Some(exit_code) = loopbox::run_loopbox_mcp_subcommand_from_args(&cli_args) {
        std::process::exit(exit_code);
    }

    suppress_noisy_h2_debug_logs();

    if let Err(err) = loopbox::ensure_installed_in_applications() {
        eprintln!("Loopbox install location warning: {err}");
    }

    if let Err(err) = loopbox::init_updater() {
        eprintln!("Loopbox updater startup warning: {err}");
    }

    let config = match loopbox::load_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Loopbox config load warning: {err}");
            loopbox::LoopboxConfig::default()
        }
    };

    match loopbox::sync_agent_api_server(&config) {
        Ok(info) if info.running => {
            let url = info
                .base_url
                .unwrap_or_else(|| "http://127.0.0.1".to_string());
            if info.auth_enabled {
                let token_hint = info.token_path.unwrap_or_else(|| "<none>".to_string());
                eprintln!("Loopbox agent API listening at {url} (token: {token_hint}).");
            } else {
                eprintln!("Loopbox agent API listening at {url} (auth disabled).");
            }
        }
        Ok(_) => {}
        Err(err) => eprintln!("Loopbox agent API startup warning: {err}"),
    }

    if let Err(err) = loopbox::sync_reverse_proxy_sidecar(&config) {
        eprintln!("Loopbox reverse proxy sidecar startup warning: {err}");
    }
    if let Err(err) = loopbox::sync_resource_metrics_sampler(&config) {
        eprintln!("Loopbox resource metrics startup warning: {err}");
    }

    match loopbox::cleanup_stale_runtime_processes() {
        Ok(removed) if removed > 0 => {
            eprintln!("Loopbox removed {removed} stale runtime pid entries on startup.");
        }
        Ok(_) => {}
        Err(err) => {
            eprintln!("Loopbox startup cleanup warning: {err}");
        }
    }

    dioxus::LaunchBuilder::desktop()
        .with_cfg(desktop_config())
        .launch(app::App);
}

#[cfg(test)]
mod tests {
    #[test]
    fn debug_desktop_builds_close_windows_so_dx_can_restart_cleanly() {
        let (close_behaviour, exits_when_last_window_closes) = super::desktop_lifecycle_mode();

        assert_eq!(
            close_behaviour,
            dioxus::desktop::WindowCloseBehaviour::WindowCloses
        );
        assert!(exits_when_last_window_closes);
    }
}
