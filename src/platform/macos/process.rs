use std::collections::HashSet;
use std::process::{Command, Stdio};

pub fn kill_process(pid: u32, signal: &str) -> Result<(), String> {
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

pub fn kill_process_group(pgid: u32, signal: &str) -> Result<(), String> {
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

pub fn kill_children(pid: u32, signal: &str) -> Result<(), String> {
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

pub fn pid_exists(pid: u32) -> bool {
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

pub fn pid_is_zombie(pid: u32) -> bool {
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

pub fn direct_child_pids(pid: u32) -> Vec<u32> {
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

pub fn process_group_pids(pgid: u32) -> Vec<u32> {
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

pub fn process_tree_pids(root_pid: u32) -> Vec<u32> {
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

pub fn process_group_is_gone(pgid: u32, observed_members: &[u32]) -> bool {
    process_group_pids(pgid).is_empty()
        && observed_members.iter().all(|pid| !pid_exists(*pid))
}
