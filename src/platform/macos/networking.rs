use std::collections::BTreeSet;
use std::fs;
use std::net::IpAddr;
use std::process::Command;

const PF_ANCHOR_NAME: &str = "com.apple/loopbox";
const PF_ANCHOR_PATH: &str = "/etc/pf.anchors/loopbox";
const PROXY_REDIRECT_SOURCE_PORT: u16 = 80;

pub fn apply_networking_script(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
    hosts_block: &str,
    fallback_port: u16,
) -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\nset -euo pipefail\n\n");
    script.push_str("# 1) Ensure loopback aliases exist\n");

    if projects.is_empty() {
        script.push_str("# No projects configured yet.\n");
    } else {
        for project in projects.values() {
            script.push_str(&format!(
                "if ! /sbin/ifconfig lo0 | /usr/bin/grep -Fq \"inet {} \"; then\n",
                project.ip
            ));
            script.push_str(&format!(
                "  sudo /sbin/ifconfig lo0 alias {} up\n",
                project.ip
            ));
            script.push_str("fi\n");
        }
    }

    script.push_str("\n# 2) Rewrite the managed loopbox block in /etc/hosts\n");
    script.push_str("tmp_file=\"$(mktemp)\"\n");
    script.push_str("/usr/bin/awk 'BEGIN {inside=0} ");
    script.push_str("$0==\"# --- loopbox begin ---\" {inside=1; next} ");
    script.push_str("$0==\"# --- loopbox end ---\" {inside=0; next} ");
    script.push_str("inside==0 {print}' /etc/hosts > \"$tmp_file\"\n");
    script.push_str("cat >> \"$tmp_file\" <<'LOOPBOX_HOSTS'\n");
    script.push_str(hosts_block);
    script.push_str("\nLOOPBOX_HOSTS\n");
    script.push_str("sudo /bin/cp \"$tmp_file\" /etc/hosts\n");
    script.push_str("rm -f \"$tmp_file\"\n");

    let ips = redirect_target_ips(projects);
    script.push_str(
        "\n# 3) Configure pf redirect for domain-only HTTP access (:80 -> proxy fallback)\n",
    );
    script.push_str("proxy_pf_tmp=\"$(mktemp)\"\n");
    script.push_str("cat > \"$proxy_pf_tmp\" <<'LOOPBOX_PF'\n");
    script.push_str(&pf_anchor_content(&ips, fallback_port));
    script.push_str("\nLOOPBOX_PF\n");
    script.push_str(&format!(
        "sudo /bin/cp \"$proxy_pf_tmp\" {}\n",
        shell_quote(PF_ANCHOR_PATH)
    ));
    script.push_str("rm -f \"$proxy_pf_tmp\"\n");
    script.push_str(&format!(
        "if /usr/bin/grep -Eq '^rdr ' {}; then\n",
        shell_quote(PF_ANCHOR_PATH)
    ));
    script.push_str("  sudo /sbin/pfctl -E >/dev/null 2>&1 || true\n");
    script.push_str(&format!(
        "  sudo /sbin/pfctl -a {} -f {}\n",
        shell_quote(PF_ANCHOR_NAME),
        shell_quote(PF_ANCHOR_PATH)
    ));
    script.push_str("else\n");
    script.push_str(&format!(
        "  sudo /sbin/pfctl -a {} -F all >/dev/null 2>&1 || true\n",
        shell_quote(PF_ANCHOR_NAME)
    ));
    script.push_str("fi\n");

    script.push_str("\n# 4) Refresh macOS DNS cache after hosts changes\n");
    script.push_str("sudo /usr/bin/dscacheutil -flushcache >/dev/null 2>&1 || true\n");
    script.push_str("sudo /usr/bin/killall -HUP mDNSResponder >/dev/null 2>&1 || true\n");
    script
}

pub fn revert_networking_script(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
) -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\nset -euo pipefail\n\n");
    script.push_str("# 1) Remove loopback aliases from current config and legacy hosts entries\n");
    for ip in projects.values().map(|project| project.ip.trim()) {
        script.push_str(&format!(
            "sudo /sbin/ifconfig lo0 -alias \"{}\" >/dev/null 2>&1 || true\n",
            ip
        ));
    }
    script.push_str("legacy_ips=\"$(/usr/bin/awk 'BEGIN {inside=0} ");
    script.push_str("$0==\"# --- loopbox begin ---\" {inside=1; next} ");
    script.push_str("$0==\"# --- loopbox end ---\" {inside=0; next} ");
    script.push_str(
        "inside==1 && $1 ~ /^127\\./ && $1 != \"127.0.0.1\" {print $1}' /etc/hosts | /usr/bin/sort -u)\"\n",
    );
    script.push_str("while IFS= read -r ip; do\n");
    script.push_str("  [ -n \"$ip\" ] || continue\n");
    script.push_str("  sudo /sbin/ifconfig lo0 -alias \"$ip\" >/dev/null 2>&1 || true\n");
    script.push_str("done <<< \"$legacy_ips\"\n");

    script.push_str("\n# 2) Remove the managed loopbox block from /etc/hosts\n");
    script.push_str("tmp_file=\"$(mktemp)\"\n");
    script.push_str("/usr/bin/awk 'BEGIN {inside=0} ");
    script.push_str("$0==\"# --- loopbox begin ---\" {inside=1; next} ");
    script.push_str("$0==\"# --- loopbox end ---\" {inside=0; next} ");
    script.push_str("inside==0 {print}' /etc/hosts > \"$tmp_file\"\n");
    script.push_str("sudo /bin/cp \"$tmp_file\" /etc/hosts\n");
    script.push_str("rm -f \"$tmp_file\"\n");

    script.push_str("\n# 3) Remove loopbox pf anchor redirect rules\n");
    script.push_str(&format!(
        "sudo /sbin/pfctl -a {} -F all >/dev/null 2>&1 || true\n",
        shell_quote(PF_ANCHOR_NAME)
    ));
    script.push_str(&format!(
        "sudo /bin/rm -f {}\n",
        shell_quote(PF_ANCHOR_PATH)
    ));

    script.push_str("\n# 4) Refresh macOS DNS cache after hosts changes\n");
    script.push_str("sudo /usr/bin/dscacheutil -flushcache >/dev/null 2>&1 || true\n");
    script.push_str("sudo /usr/bin/killall -HUP mDNSResponder >/dev/null 2>&1 || true\n");
    script
}

pub fn loopback_alias_present(ip: &str) -> bool {
    let output = Command::new("/sbin/ifconfig").arg("lo0").output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains(&format!("inet {ip} "))
}

pub fn proxy_redirect_configured_on_system(
    projects: &std::collections::BTreeMap<String, crate::loopbox::ProjectConfig>,
    fallback_port: u16,
) -> bool {
    let ips = redirect_target_ips(projects);
    if ips.is_empty() {
        return true;
    }

    let Ok(anchor_content) = fs::read_to_string(PF_ANCHOR_PATH) else {
        return false;
    };
    let has_rdr = anchor_content.lines().any(|line| {
        let clean = line.trim();
        clean.starts_with("rdr ")
            && clean.contains(" on lo0 ")
            && clean.contains(&format!(" port {PROXY_REDIRECT_SOURCE_PORT}"))
            && clean.contains("-> 127.0.0.1")
            && clean.contains(&format!(" port {fallback_port}"))
    });
    if !has_rdr {
        return false;
    }
    for ip in &ips {
        if !anchor_content.contains(ip) {
            return false;
        }
    }
    true
}

pub fn dns_flush_command() -> &'static str {
    "sudo dscacheutil -flushcache && sudo killall -HUP mDNSResponder"
}

pub fn loopback_interface_label() -> &'static str {
    "lo0"
}

pub fn proxy_redirect_label() -> &'static str {
    "pf redirect"
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

fn pf_anchor_content(ips: &[String], fallback_port: u16) -> String {
    let mut lines = vec![
        "# loopbox managed pf anchor".to_string(),
        "# Enables domain-only access by redirecting sandbox-ip:80 to the internal proxy fallback port.".to_string(),
    ];

    if ips.is_empty() {
        lines.push("# no networked services configured".to_string());
        return lines.join("\n");
    }

    let ip_list = ips.join(", ");
    lines.push(format!(
        "rdr pass on lo0 inet proto tcp from any to {{ {ip_list} }} port {PROXY_REDIRECT_SOURCE_PORT} -> 127.0.0.1 port {fallback_port}"
    ));
    lines.join("\n")
}

fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}
