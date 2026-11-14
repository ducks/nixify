use crate::inventory::Inventory;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    net::IpAddr,
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkInventory {
    pub evidence: Vec<String>,
    pub interfaces: Option<InterfaceSummary>,
    pub dns: Option<DnsSummary>,
    /// Installed package names only; this does not establish runtime activity.
    pub firewall_packages: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InterfaceSummary {
    pub loopback: u32,
    pub wireless: u32,
    pub ethernet_or_virtual: u32,
    pub other: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DnsSummary {
    pub loopback_nameservers: u32,
    pub other_nameservers: u32,
    pub unrecognized_nameservers: u32,
}

pub fn parse_resolver(text: &str) -> DnsSummary {
    let mut result = DnsSummary::default();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("nameserver") {
            continue;
        }
        let address = fields.next().unwrap_or("").split('%').next().unwrap_or("");
        match address.parse::<IpAddr>() {
            Ok(ip) if ip.is_loopback() => result.loopback_nameservers += 1,
            Ok(_) => result.other_nameservers += 1,
            Err(_) => result.unrecognized_nameservers += 1,
        }
    }
    result
}

fn bounded_text(path: &Path, max: u64) -> Option<String> {
    let mut result = String::new();
    File::open(path)
        .ok()?
        .take(max + 1)
        .read_to_string(&mut result)
        .ok()?;
    if result.len() as u64 > max {
        return None;
    }
    Some(result)
}
fn resolver() -> Option<DnsSummary> {
    // Do not follow an unexpected link into a user's home or arbitrary files.
    let path = fs::canonicalize("/etc/resolv.conf").ok()?;
    if !["/etc", "/run/systemd/resolve", "/run/NetworkManager"]
        .iter()
        .any(|root| path.starts_with(root))
    {
        return None;
    }
    bounded_text(&path, 64 * 1024).map(|s| parse_resolver(&s))
}

pub fn interface_summary(root: &Path) -> std::io::Result<InterfaceSummary> {
    let mut result = InterfaceSummary::default();
    for (index, entry) in fs::read_dir(root)?.enumerate() {
        if index >= 4096 {
            return Err(std::io::Error::other("too many network interfaces"));
        }
        let entry = entry?;
        let path = entry.path();
        let kind = bounded_text(&path.join("type"), 64)
            .ok_or_else(|| std::io::Error::other("cannot inspect interface type"))?;
        let kind: u32 = kind
            .trim()
            .parse()
            .map_err(|_| std::io::Error::other("invalid interface type"))?;
        if kind == 772 {
            result.loopback += 1;
            continue;
        }
        let wireless = match fs::metadata(path.join("wireless")) {
            Ok(metadata) => metadata.is_dir(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if wireless {
            result.wireless += 1;
        } else if kind == 1 {
            result.ethernet_or_virtual += 1;
        } else {
            result.other += 1;
        }
    }
    Ok(result)
}
pub fn scan(inventory: &Inventory) -> NetworkInventory {
    let names: BTreeSet<&str> = inventory.packages.iter().map(|p| p.name.as_str()).collect();
    let firewall_packages = ["firewalld", "ufw", "nftables", "iptables", "iptables-nft"]
        .into_iter()
        .filter(|name| names.contains(name))
        .map(str::to_owned)
        .collect();
    NetworkInventory {
        evidence: vec![
            "/sys/class/net interface types (counts only)".into(),
            "/etc/resolv.conf nameserver categories (no addresses)".into(),
            "pacman installed package names".into(),
        ],
        interfaces: interface_summary(Path::new("/sys/class/net")).ok(),
        dns: resolver(),
        firewall_packages,
    }
}
