use super::*;
use crate::platform::process as platform;

pub(super) fn terminate_pid_if_alive(pid: u32, process_group_leader: bool) -> Result<bool, String> {
    if pid == 0 {
        return Err("Refusing to terminate invalid pid 0.".to_string());
    }

    if process_group_leader {
        let mut observed_group_members = platform::process_group_pids(pid);
        if platform::pid_exists(pid) && !observed_group_members.contains(&pid) {
            observed_group_members.push(pid);
        }
        if observed_group_members.is_empty() {
            return Ok(false);
        }

        let mut term_error = platform::kill_process_group(pid, "TERM").err();
        if let Err(err) = platform::kill_process(pid, "TERM") {
            if term_error.is_none() {
                term_error = Some(err);
            }
        }
        for _ in 0..8 {
            if platform::process_group_is_gone(pid, &observed_group_members) {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(80));
        }

        let mut kill_error = platform::kill_process_group(pid, "KILL").err();
        if let Err(err) = platform::kill_process(pid, "KILL") {
            if kill_error.is_none() {
                kill_error = Some(err);
            }
        }
        for _ in 0..6 {
            if platform::process_group_is_gone(pid, &observed_group_members) {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(50));
        }

        let detail = kill_error
            .or(term_error)
            .unwrap_or_else(|| "No signal delivery error was reported.".to_string());
        return Err(format!(
            "Process group rooted at {pid} is still alive after TERM/KILL. {detail}"
        ));
    }

    if !platform::pid_exists(pid) {
        return Ok(false);
    }

    let targets = platform::process_tree_pids(pid);
    let mut legacy_term_error = None;
    if let Err(err) = platform::kill_children(pid, "TERM") {
        legacy_term_error = Some(err);
    }
    let term_error = signal_pid_set(&targets, "TERM");
    for _ in 0..8 {
        if all_pids_gone(&targets) {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(80));
    }

    let mut legacy_kill_error = None;
    if let Err(err) = platform::kill_children(pid, "KILL") {
        legacy_kill_error = Some(err);
    }
    let kill_error = signal_pid_set(&targets, "KILL");
    for _ in 0..6 {
        if all_pids_gone(&targets) {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(50));
    }

    let detail = kill_error
        .or(legacy_kill_error)
        .or(term_error)
        .or(legacy_term_error)
        .unwrap_or_else(|| "No signal delivery error was reported.".to_string());
    Err(format!(
        "Process tree rooted at {pid} is still alive after TERM/KILL. {detail}"
    ))
}

fn all_pids_gone(pids: &[u32]) -> bool {
    pids.iter().all(|pid| !platform::pid_exists(*pid))
}

fn signal_pid_set(pids: &[u32], signal: &str) -> Option<String> {
    let mut first_error = None;
    for pid in pids.iter().rev() {
        if let Err(err) = platform::kill_process(*pid, signal) {
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }
    first_error
}

pub(super) fn pid_exists(pid: u32) -> bool {
    platform::pid_exists(pid)
}
