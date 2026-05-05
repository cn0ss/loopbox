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
    if status.success() || process_group_pids(pgid).is_empty() {
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
    process_group_pids(pgid).is_empty() && observed_members.iter().all(|pid| !pid_exists(*pid))
}

pub fn process_tree_resource_usage(root_pid: u32) -> Result<ProcessResourceUsage, String> {
    let pids = process_tree_pids(root_pid);
    if pids.is_empty() {
        return Err(format!("No live process tree found for pid {root_pid}."));
    }

    let output = Command::new("/bin/ps")
        .env("LC_ALL", "C")
        .arg("-o")
        .arg("pcpu=")
        .arg("-o")
        .arg("rss=")
        .arg("-p")
        .arg(
            pids.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        )
        .output()
        .map_err(|err| format!("Failed to inspect resource usage for pid {root_pid}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Resource usage inspection failed for pid {root_pid}."
        ));
    }
    process_resource_usage_from_ps_output(&String::from_utf8_lossy(&output.stdout))
}

fn process_resource_usage_from_ps_output(stdout: &str) -> Result<ProcessResourceUsage, String> {
    let mut cpu_percent = 0.0_f64;
    let mut memory_bytes = 0_u64;
    let mut process_count = 0_usize;

    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(cpu_raw) = parts.next() else {
            continue;
        };
        let Some(rss_raw) = parts.next() else {
            continue;
        };
        let cpu = parse_ps_cpu_percent(cpu_raw)
            .ok_or_else(|| format!("Invalid CPU value '{cpu_raw}' in ps output."))?;
        let rss_kb = rss_raw
            .parse::<u64>()
            .map_err(|_| format!("Invalid RSS value '{rss_raw}' in ps output."))?;
        cpu_percent += cpu;
        memory_bytes = memory_bytes.saturating_add(rss_kb.saturating_mul(1024));
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

fn parse_ps_cpu_percent(raw: &str) -> Option<f64> {
    raw.trim().replace(',', ".").parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::process_resource_usage_from_ps_output;

    #[test]
    fn process_resource_usage_from_ps_output_accepts_localized_cpu_decimal_comma() {
        let usage = process_resource_usage_from_ps_output("  0,0  1024\n 12,5 2048\n")
            .expect("localized ps output should parse");

        assert_eq!(usage.cpu_percent, 12.5);
        assert_eq!(usage.memory_bytes, 3 * 1024 * 1024);
        assert_eq!(usage.process_count, 2);
    }
}
