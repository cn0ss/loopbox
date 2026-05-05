use std::collections::HashSet;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessResourceUsage {
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub process_count: usize,
}

pub fn kill_process(pid: u32, signal: &str) -> Result<(), String> {
    if pid == 0 {
        return Err("Refusing to signal invalid pid 0.".to_string());
    }

    let mut cmd = Command::new("taskkill");
    cmd.arg("/PID").arg(pid.to_string());
    if signal == "KILL" || signal == "9" {
        cmd.arg("/F");
    }

    let status = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| format!("Failed to execute taskkill for pid {pid}: {err}"))?;

    if status.success() || !pid_exists(pid) {
        Ok(())
    } else {
        Err(format!("taskkill failed for pid {pid}."))
    }
}

pub fn kill_process_group(pgid: u32, signal: &str) -> Result<(), String> {
    if pgid == 0 {
        return Err("Refusing to signal invalid process group 0.".to_string());
    }

    // Windows has no process groups. Kill children first, then the process itself.
    let _ = kill_children(pgid, signal);
    kill_process(pgid, signal)
}

pub fn kill_children(pid: u32, signal: &str) -> Result<(), String> {
    if pid == 0 {
        return Err("Refusing to signal children of invalid pid 0.".to_string());
    }

    let children = direct_child_pids(pid);
    let mut last_err: Option<String> = None;
    for child in children {
        if let Err(err) = kill_process(child, signal) {
            last_err = Some(err);
        }
    }

    match last_err {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

pub fn pid_exists(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    let output = Command::new("tasklist")
        .arg("/FI")
        .arg(format!("PID eq {pid}"))
        .arg("/NH")
        .output();

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // tasklist prints "INFO: No tasks are running..." when pid is not found.
    // When found, it prints a line containing the PID number.
    stdout.contains(&format!(" {pid} "))
}

pub fn pid_is_zombie(pid: u32) -> bool {
    // Windows does not have zombie processes.
    let _ = pid;
    false
}

pub fn direct_child_pids(pid: u32) -> Vec<u32> {
    let output = Command::new("wmic")
        .arg("process")
        .arg("where")
        .arg(format!("(ParentProcessId={pid})"))
        .arg("get")
        .arg("ProcessId")
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
    // Windows has no process groups. Return just the pid if it exists.
    if pid_exists(pgid) {
        vec![pgid]
    } else {
        Vec::new()
    }
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
    process_group_pids(pgid).is_empty() && observed_members.iter().all(|pid| !pid_exists(*pid))
}

pub fn process_tree_resource_usage(root_pid: u32) -> Result<ProcessResourceUsage, String> {
    let pids = process_tree_pids(root_pid);
    if pids.is_empty() {
        return Err(format!("No live process tree found for pid {root_pid}."));
    }

    let output = Command::new("wmic")
        .args([
            "path",
            "Win32_PerfFormattedData_PerfProc_Process",
            "get",
            "IDProcess,PercentProcessorTime,WorkingSet",
            "/format:csv",
        ])
        .output()
        .map_err(|err| format!("Failed to inspect resource usage for pid {root_pid}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Resource usage inspection failed for pid {root_pid}."
        ));
    }
    process_resource_usage_from_wmic_output(&String::from_utf8_lossy(&output.stdout), &pids)
}

fn process_resource_usage_from_wmic_output(
    stdout: &str,
    pids: &[u32],
) -> Result<ProcessResourceUsage, String> {
    let pid_set = pids.iter().copied().collect::<HashSet<_>>();
    let mut cpu_percent = 0.0_f64;
    let mut memory_bytes = 0_u64;
    let mut process_count = 0_usize;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Node,") {
            continue;
        }
        let parts = trimmed.split(',').collect::<Vec<_>>();
        if parts.len() < 4 {
            continue;
        }
        let pid = parts[1].trim().parse::<u32>().ok();
        let Some(pid) = pid.filter(|pid| pid_set.contains(pid)) else {
            continue;
        };
        let _ = pid;
        let cpu = parts[2].trim().parse::<f64>().unwrap_or(0.0);
        let working_set = parts[3].trim().parse::<u64>().unwrap_or(0);
        cpu_percent += cpu;
        memory_bytes = memory_bytes.saturating_add(working_set);
        process_count = process_count.saturating_add(1);
    }

    if process_count == 0 {
        return Err("Resource usage inspection returned no process rows.".to_string());
    }

    Ok(ProcessResourceUsage {
        cpu_percent,
        memory_bytes,
        process_count,
    })
}
