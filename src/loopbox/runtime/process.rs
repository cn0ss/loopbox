use super::*;

pub(super) fn terminate_pid_if_alive(pid: u32, process_group_leader: bool) -> Result<bool, String> {
    if pid == 0 {
        return Err("Refusing to terminate invalid pid 0.".to_string());
    }

    if process_group_leader {
        let mut observed_group_members = process_group_pids(pid);
        if pid_exists(pid) && !observed_group_members.contains(&pid) {
            observed_group_members.push(pid);
        }
        if observed_group_members.is_empty() {
            return Ok(false);
        }

        let mut term_error = run_kill_signal_group(pid, "TERM").err();
        if let Err(err) = run_kill_signal(pid, "TERM") {
            if term_error.is_none() {
                term_error = Some(err);
            }
        }
        for _ in 0..8 {
            if process_group_is_gone(pid, &observed_group_members) {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(80));
        }

        let mut kill_error = run_kill_signal_group(pid, "KILL").err();
        if let Err(err) = run_kill_signal(pid, "KILL") {
            if kill_error.is_none() {
                kill_error = Some(err);
            }
        }
        for _ in 0..6 {
            if process_group_is_gone(pid, &observed_group_members) {
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

    if !pid_exists(pid) {
        return Ok(false);
    }

    let targets = process_tree_pids(pid);
    let mut legacy_term_error = None;
    #[cfg(unix)]
    {
        if let Err(err) = run_kill_children_signal(pid, "TERM") {
            legacy_term_error = Some(err);
        }
    }
    let term_error = signal_pid_set(&targets, "TERM");
    for _ in 0..8 {
        if all_pids_gone(&targets) {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(80));
    }

    let mut legacy_kill_error = None;
    #[cfg(unix)]
    {
        if let Err(err) = run_kill_children_signal(pid, "KILL") {
            legacy_kill_error = Some(err);
        }
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

#[cfg(unix)]
fn process_group_is_gone(pgid: u32, observed_members: &[u32]) -> bool {
    process_group_pids(pgid).is_empty() && all_pids_gone(observed_members)
}

#[cfg(not(unix))]
fn process_group_is_gone(_pgid: u32, observed_members: &[u32]) -> bool {
    all_pids_gone(observed_members)
}

fn all_pids_gone(pids: &[u32]) -> bool {
    pids.iter().all(|pid| !pid_exists(*pid))
}

fn signal_pid_set(pids: &[u32], signal: &str) -> Option<String> {
    let mut first_error = None;
    for pid in pids.iter().rev() {
        if let Err(err) = run_kill_signal(*pid, signal) {
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }
    first_error
}

#[cfg(unix)]
fn run_kill_signal(pid: u32, signal: &str) -> Result<(), String> {
    if pid == 0 {
        return Err("Refusing to signal invalid pid 0.".to_string());
    }

    let status = Command::new("/bin/kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| format!("Failed to execute kill -{signal} for pid {pid}: {err}"))?;
    if status.success() || !pid_exists(pid) {
        Ok(())
    } else {
        Err(format!("kill -{signal} failed for pid {pid}."))
    }
}

#[cfg(not(unix))]
fn run_kill_signal(_pid: u32, _signal: &str) -> Result<(), String> {
    Err("Process termination is only supported on Unix targets.".to_string())
}

#[cfg(unix)]
fn run_kill_signal_group(pgid: u32, signal: &str) -> Result<(), String> {
    if pgid == 0 {
        return Err("Refusing to signal invalid process group 0.".to_string());
    }

    let status = Command::new("/bin/kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(format!("-{pgid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| {
            format!("Failed to execute kill -{signal} for process group {pgid}: {err}")
        })?;
    if status.success() || !pid_exists(pgid) {
        Ok(())
    } else {
        Err(format!("kill -{signal} failed for process group {pgid}."))
    }
}

#[cfg(not(unix))]
fn run_kill_signal_group(_pgid: u32, _signal: &str) -> Result<(), String> {
    Err("Process group termination is only supported on Unix targets.".to_string())
}

#[cfg(unix)]
fn run_kill_children_signal(pid: u32, signal: &str) -> Result<(), String> {
    if pid == 0 {
        return Err("Refusing to signal children of invalid pid 0.".to_string());
    }

    let status = Command::new("/usr/bin/pkill")
        .arg(format!("-{signal}"))
        .arg("-P")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| format!("Failed to execute pkill -{signal} -P {pid}: {err}"))?;

    if status.success() || status.code() == Some(1) {
        Ok(())
    } else {
        Err(format!("pkill -{signal} -P {pid} failed."))
    }
}

#[cfg(unix)]
fn process_tree_pids(root_pid: u32) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut queue = vec![root_pid];
    let mut ordered = Vec::new();
    while let Some(pid) = queue.pop() {
        if !seen.insert(pid) {
            continue;
        }
        ordered.push(pid);
        for child_pid in direct_child_pids(pid) {
            if !seen.contains(&child_pid) {
                queue.push(child_pid);
            }
        }
    }
    ordered
}

#[cfg(not(unix))]
fn process_tree_pids(root_pid: u32) -> Vec<u32> {
    vec![root_pid]
}

#[cfg(unix)]
fn direct_child_pids(pid: u32) -> Vec<u32> {
    let output = Command::new("/usr/bin/pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(unix)]
fn process_group_pids(pgid: u32) -> Vec<u32> {
    let output = Command::new("/usr/bin/pgrep")
        .arg("-g")
        .arg(pgid.to_string())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(not(unix))]
fn process_group_pids(_pgid: u32) -> Vec<u32> {
    Vec::new()
}

#[cfg(unix)]
pub(super) fn pid_exists(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success() && !pid_is_zombie(pid))
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(super) fn pid_exists(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn pid_is_zombie(pid: u32) -> bool {
    let output = Command::new("/bin/ps")
        .arg("-o")
        .arg("stat=")
        .arg("-p")
        .arg(pid.to_string())
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stat = String::from_utf8_lossy(&output.stdout);
    stat.trim_start().starts_with('Z')
}

#[cfg(not(unix))]
fn pid_is_zombie(_pid: u32) -> bool {
    false
}
