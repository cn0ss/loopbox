use std::collections::BTreeSet;
use std::net::IpAddr;
use std::process::Command;

const LOOPBACK_ADAPTER: &str = "Loopback Pseudo-Interface 1";
const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
const PROXY_REDIRECT_SOURCE_PORT: u16 = 80;

pub fn apply_networking_script(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
    hosts_block: &str,
    fallback_port: u16,
) -> String {
    let mut script = String::new();
    script.push_str("@echo off\r\n");

    script.push_str("REM 1) Ensure loopback aliases exist\r\n");
    if projects.is_empty() {
        script.push_str("REM No projects configured yet.\r\n");
    } else {
        for project in projects.values() {
            let ip = project.ip.trim();
            script.push_str(&format!(
                "netsh interface ipv4 add address \"{}\" {} 255.255.255.255 >nul 2>&1\r\n",
                LOOPBACK_ADAPTER, ip
            ));
        }
    }

    script.push_str("\r\nREM 2) Rewrite the managed loopbox block in hosts file\r\n");
    // Use PowerShell to do the hosts file manipulation since batch has limited text processing.
    let escaped_hosts_block = hosts_block.replace('\'', "''");
    script.push_str(&format!(
        "powershell -Command \"\
         $hostsPath = '{hosts_path}';\
         $content = Get-Content $hostsPath -Raw -ErrorAction SilentlyContinue;\
         if (-not $content) {{ $content = '' }};\
         $content = $content -replace '(?ms)# --- loopbox begin ---.*?# --- loopbox end ---\\r?\\n?', '';\
         $content = $content.TrimEnd() + \\\"`r`n{block}\\\";\
         Set-Content -Path $hostsPath -Value $content -Encoding ASCII\
         \"\r\n",
        hosts_path = HOSTS_PATH,
        block = escaped_hosts_block.replace('"', "`\"").replace('\n', "`r`n"),
    ));

    let ips = redirect_target_ips(projects);

    script.push_str("\r\nREM 3) Configure port proxy for domain-only HTTP access (:80 -> proxy fallback)\r\n");
    for ip in &ips {
        script.push_str(&format!(
            "netsh interface portproxy add v4tov4 listenport={} listenaddress={} connectport={} connectaddress=127.0.0.1 >nul 2>&1\r\n",
            PROXY_REDIRECT_SOURCE_PORT, ip, fallback_port
        ));
    }

    script.push_str("\r\nREM 4) Flush DNS cache\r\n");
    script.push_str("ipconfig /flushdns >nul 2>&1\r\n");
    script
}

pub fn revert_networking_script(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
) -> String {
    let mut script = String::new();
    script.push_str("@echo off\r\n");

    script.push_str("REM 1) Remove loopback aliases\r\n");
    for project in projects.values() {
        let ip = project.ip.trim();
        script.push_str(&format!(
            "netsh interface ipv4 delete address \"{}\" {} >nul 2>&1\r\n",
            LOOPBACK_ADAPTER, ip
        ));
    }

    script.push_str("\r\nREM 2) Remove port proxy rules\r\n");
    let ips = redirect_target_ips(projects);
    for ip in &ips {
        script.push_str(&format!(
            "netsh interface portproxy delete v4tov4 listenport={} listenaddress={} >nul 2>&1\r\n",
            PROXY_REDIRECT_SOURCE_PORT, ip
        ));
    }

    script.push_str("\r\nREM 3) Remove the managed loopbox block from hosts file\r\n");
    script.push_str(&format!(
        "powershell -Command \"\
         $hostsPath = '{hosts_path}';\
         $content = Get-Content $hostsPath -Raw -ErrorAction SilentlyContinue;\
         if ($content) {{\
         $content = $content -replace '(?ms)# --- loopbox begin ---.*?# --- loopbox end ---\\r?\\n?', '';\
         Set-Content -Path $hostsPath -Value $content.TrimEnd() -Encoding ASCII\
         }}\
         \"\r\n",
        hosts_path = HOSTS_PATH,
    ));

    script.push_str("\r\nREM 4) Flush DNS cache\r\n");
    script.push_str("ipconfig /flushdns >nul 2>&1\r\n");
    script
}

pub fn loopback_alias_present(ip: &str) -> bool {
    let output = Command::new("netsh")
        .arg("interface")
        .arg("ipv4")
        .arg("show")
        .arg("addresses")
        .arg(LOOPBACK_ADAPTER)
        .output();

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains(ip)
}

pub fn proxy_redirect_configured_on_system(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
    fallback_port: u16,
) -> bool {
    let ips = redirect_target_ips(projects);
    if ips.is_empty() {
        return true;
    }

    let output = Command::new("netsh")
        .arg("interface")
        .arg("portproxy")
        .arg("show")
        .arg("v4tov4")
        .output();

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for ip in &ips {
        // Expect a line containing the listen address, source port, and connect port
        let has_rule = stdout.lines().any(|line| {
            line.contains(ip)
                && line.contains(&PROXY_REDIRECT_SOURCE_PORT.to_string())
                && line.contains(&fallback_port.to_string())
                && line.contains("127.0.0.1")
        });
        if !has_rule {
            return false;
        }
    }
    true
}

pub fn dns_flush_command() -> &'static str {
    "ipconfig /flushdns"
}

fn redirect_target_ips(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
) -> Vec<String> {
    let mut ips = BTreeSet::new();
    for project in projects.values() {
        if !project
            .services
            .iter()
            .any(|service| !crate::loopbox::service_ports(service).is_empty())
        {
            continue;
        }
        let candidate = project.ip.trim();
        if candidate.parse::<IpAddr>().is_ok() {
            ips.insert(candidate.to_string());
        }
    }
    ips.into_iter().collect()
}
