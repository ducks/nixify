use crate::inventory::{valid_package_name, Inventory, Package, Source};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;

const MAX_FILE: u64 = 64 * 1024;
const MAX_OUTPUT: u64 = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("{0}")]
    Invalid(String),
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}
fn io_error(context: &str, source: std::io::Error) -> ScanError {
    ScanError::Io {
        context: context.into(),
        source,
    }
}

pub fn distro_id(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ID="))
        .next_back()
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
}
fn os_release() -> Result<String, ScanError> {
    let mut text = String::new();
    File::open("/etc/os-release")
        .map_err(|e| io_error("cannot read /etc/os-release", e))?
        .take(MAX_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|e| io_error("cannot read /etc/os-release", e))?;
    if text.len() as u64 > MAX_FILE {
        return Err(ScanError::Invalid(
            "/etc/os-release exceeds size limit".into(),
        ));
    }
    Ok(text)
}
fn read_bounded<R: Read>(reader: R) -> std::io::Result<Vec<u8>> {
    let mut b = Vec::new();
    reader.take(MAX_OUTPUT + 1).read_to_end(&mut b)?;
    Ok(b)
}
pub fn run_bounded(program: &str, args: &[&str], empty_ok: bool) -> Result<String, ScanError> {
    let arg = if program == "pacman" {
        args.join(" ")
    } else {
        "[arguments omitted]".into()
    };
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| io_error(&format!("could not start {program}"), e))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ScanError::Invalid("subprocess stdout unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ScanError::Invalid("subprocess stderr unavailable".into()))?;
    let out = thread::spawn(move || read_bounded(stdout));
    let err = thread::spawn(move || read_bounded(stderr));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| io_error("waiting for subprocess", e))?
        {
            break status;
        }
        if start.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(ScanError::Invalid(format!("{program} {arg} timed out")));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let out = out
        .join()
        .map_err(|_| ScanError::Invalid("subprocess output reader failed".into()))?
        .map_err(|e| io_error("reading subprocess stdout", e))?;
    let err = err
        .join()
        .map_err(|_| ScanError::Invalid("subprocess error reader failed".into()))?
        .map_err(|e| io_error("reading subprocess stderr", e))?;
    if out.len() as u64 > MAX_OUTPUT || err.len() as u64 > MAX_OUTPUT {
        return Err(ScanError::Invalid(format!(
            "{program} {arg} output exceeds size limit"
        )));
    }
    if !(status.success()
        || empty_ok && status.code() == Some(1) && out.is_empty() && err.is_empty())
    {
        return Err(ScanError::Invalid(format!(
            "{program} {arg} failed ({}): {}",
            status,
            String::from_utf8_lossy(&err).trim()
        )));
    }
    String::from_utf8(out)
        .map_err(|_| ScanError::Invalid(format!("{program} {arg} returned invalid UTF-8")))
}
fn names(text: &str) -> Result<BTreeSet<String>, ScanError> {
    let mut set = BTreeSet::new();
    for name in text.lines().filter(|line| !line.is_empty()) {
        if !valid_package_name(name) || !set.insert(name.to_string()) {
            return Err(ScanError::Invalid(format!(
                "invalid or duplicate pacman package name: {name:?}"
            )));
        }
    }
    Ok(set)
}
pub fn inventory(
    os: &str,
    architecture: &str,
    all: &str,
    explicit: &str,
    foreign: &str,
) -> Result<Inventory, ScanError> {
    let id =
        distro_id(os).ok_or_else(|| ScanError::Invalid("/etc/os-release lacks an ID".into()))?;
    if id != "manjaro" {
        return Err(ScanError::Invalid(format!(
            "unsupported distribution ID {id:?}; this scanner supports Manjaro only"
        )));
    }
    let explicit = names(explicit)?;
    let foreign = names(foreign)?;
    let mut packages = BTreeMap::new();
    for line in all.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or("");
        let version = fields.next().unwrap_or("");
        if !valid_package_name(name)
            || version.is_empty()
            || fields.next().is_some()
            || packages.contains_key(name)
        {
            return Err(ScanError::Invalid(format!(
                "invalid pacman -Q record: {line:?}"
            )));
        }
        packages.insert(name.to_owned(), version.to_owned());
    }
    if packages.is_empty() {
        return Err(ScanError::Invalid("pacman -Q returned no packages".into()));
    }
    if explicit
        .iter()
        .chain(foreign.iter())
        .any(|name| !packages.contains_key(name))
    {
        return Err(ScanError::Invalid("pacman query results disagree; package database may have changed during scanning; retry".into()));
    }
    let packages = packages
        .into_iter()
        .map(|(name, version)| Package {
            explicitly_installed: explicit.contains(&name),
            foreign: foreign.contains(&name),
            name,
            version,
            evidence: vec![
                "pacman -Q".into(),
                "pacman -Qqe".into(),
                "pacman -Qqm".into(),
            ],
        })
        .collect();
    Ok(Inventory { schema_version: 1, source: Source { distribution: id, architecture: architecture.into(), evidence: "/etc/os-release:ID".into() }, packages, service_scopes: Vec::new(), desktop: None, hardware: None, networking: None, config_evidence: None, warnings: vec!["Foreign means absent from configured sync databases; it does not establish AUR origin.".into(), "Hardware and network settings require manual migration review.".into()] })
}
pub fn scan(requested: Option<&str>) -> Result<Inventory, ScanError> {
    let os = os_release()?;
    if let Some(requested) = requested {
        if requested != "manjaro" {
            return Err(ScanError::Invalid(format!(
                "unsupported scanner: {requested}"
            )));
        }
    }
    if distro_id(&os).as_deref() != Some("manjaro") {
        return Err(ScanError::Invalid(
            "this host is not identified as Manjaro in /etc/os-release".into(),
        ));
    }
    let all = run_bounded("pacman", &["-Q"], false)?;
    let explicit = run_bounded("pacman", &["-Qqe"], true)?;
    let foreign = run_bounded("pacman", &["-Qqm"], true)?;
    let mut inventory = inventory(&os, std::env::consts::ARCH, &all, &explicit, &foreign)?;
    inventory.service_scopes = crate::systemd::scan();
    for scope in &inventory.service_scopes {
        if !scope.unit_files_available || !scope.runtime_available {
            inventory.warnings.push(format!(
                "{:?} systemd scope: unit files {}, runtime {}; unavailable data is unknown.",
                scope.scope,
                if scope.unit_files_available {
                    "available"
                } else {
                    "unavailable"
                },
                if scope.runtime_available {
                    "available"
                } else {
                    "unavailable"
                }
            ));
        }
    }
    inventory.desktop = Some(crate::desktop::scan(&inventory));
    let hardware = crate::hardware::scan();
    if hardware.pci_devices.is_none() {
        inventory
            .warnings
            .push("PCI inventory unavailable; GPU and network adapter evidence is unknown.".into());
    }
    if hardware.block_devices.is_none() {
        inventory
            .warnings
            .push("Block device inventory unavailable; filesystems require manual review.".into());
    }
    inventory.hardware = Some(hardware);
    let network = crate::network::scan(&inventory);
    if network.interfaces.is_none() {
        inventory
            .warnings
            .push("Network interface inventory unavailable; interface evidence is unknown.".into());
    }
    if network.dns.is_none() {
        inventory
            .warnings
            .push("Resolver evidence unavailable; DNS configuration is unknown.".into());
    }
    inventory.networking = Some(network);
    inventory.config_evidence = Some(crate::config_evidence::scan(&inventory));
    Ok(inventory)
}
