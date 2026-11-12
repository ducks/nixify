use crate::manjaro::run_bounded;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hardware {
    pub evidence: Vec<String>,
    pub cpu_model: Option<String>,
    /// Presence of EFI firmware interfaces is evidence of UEFI. Absence is unknown.
    pub uefi_visible: Option<bool>,
    /// None means unavailable or malformed output, not no hardware.
    pub pci_devices: Option<Vec<PciDevice>>,
    pub block_devices: Option<Vec<BlockDevice>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PciDevice {
    pub category: String,
    pub vendor: String,
    pub model: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDevice {
    pub kind: String,
    pub filesystem: Option<String>,
    pub size_bytes: u64,
    /// Only /, /boot, /boot/efi and /efi are retained.
    pub system_mounts: Vec<String>,
}
#[derive(Deserialize)]
struct Lsblk {
    blockdevices: Vec<LsblkDevice>,
}
#[derive(Deserialize)]
struct LsblkDevice {
    #[serde(rename = "type")]
    kind: String,
    fstype: Option<String>,
    size: Option<u64>,
    mountpoints: Option<Vec<Option<String>>>,
}
fn safe_label(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

/// Parse the first four quoted fields of `lspci -mm -nn`, rejecting malformed
/// escapes rather than retaining untrusted lines verbatim. PCI addresses and
/// subsystem fields are intentionally discarded.
fn quoted_fields(line: &str) -> Option<Vec<String>> {
    let mut chars = line.chars().peekable();
    for c in chars.by_ref() {
        if c.is_whitespace() {
            break;
        }
    }
    let mut fields = Vec::new();
    while fields.len() < 3 {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.next()? != '"' {
            return None;
        }
        let mut field = String::new();
        let mut closed = false;
        while let Some(c) = chars.next() {
            if c == '"' {
                closed = true;
                break;
            }
            if c == '\\' {
                let next = chars.next()?;
                if next != '\\' && next != '"' {
                    return None;
                }
                field.push(next);
            } else {
                field.push(c);
            }
        }
        if !closed || !safe_label(&field) {
            return None;
        }
        fields.push(field);
    }
    Some(fields)
}
fn class_code(class: &str) -> Option<&str> {
    let code = class.rsplit_once('[')?.1.strip_suffix(']')?;
    if code.len() == 4 && code.bytes().all(|c| c.is_ascii_hexdigit()) {
        Some(code)
    } else {
        None
    }
}
pub fn parse_pci(text: &str) -> Result<Vec<PciDevice>, String> {
    let mut devices = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let fields = quoted_fields(line).ok_or("malformed lspci machine-readable output")?;
        let code = class_code(&fields[0]).ok_or("missing PCI class code")?;
        let category = match &code[..2] {
            "03" => "display",
            "02" => "network",
            _ => continue,
        };
        devices.push(PciDevice {
            category: category.into(),
            vendor: fields[1].clone(),
            model: fields[2].clone(),
        });
    }
    devices.sort_by(|a, b| {
        (&a.category, &a.vendor, &a.model).cmp(&(&b.category, &b.vendor, &b.model))
    });
    Ok(devices)
}
pub fn parse_lsblk(text: &str) -> Result<Vec<BlockDevice>, String> {
    let raw: Lsblk = serde_json::from_str(text).map_err(|_| "malformed lsblk JSON")?;
    let mut devices = Vec::new();
    for d in raw.blockdevices {
        if d.kind == "loop" {
            continue;
        }
        if d.kind.is_empty()
            || d.kind.len() > 32
            || !d
                .kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err("invalid block device type".into());
        }
        if d.fstype.as_ref().is_some_and(|s| {
            !safe_label(s)
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        }) {
            return Err("invalid filesystem type".into());
        }
        let size = d.size.ok_or("block device size missing")?;
        let mut mounts: Vec<_> = d
            .mountpoints
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .filter(|m| ["/", "/boot", "/boot/efi", "/efi"].contains(&m.as_str()))
            .collect();
        mounts.sort();
        mounts.dedup();
        devices.push(BlockDevice {
            kind: d.kind,
            filesystem: d.fstype,
            size_bytes: size,
            system_mounts: mounts,
        });
    }
    devices.sort_by(|a, b| {
        (&a.kind, &a.filesystem, a.size_bytes, &a.system_mounts).cmp(&(
            &b.kind,
            &b.filesystem,
            b.size_bytes,
            &b.system_mounts,
        ))
    });
    Ok(devices)
}
pub fn parse_cpu(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| {
            if ["model name", "Hardware"].contains(&key.trim()) && safe_label(value.trim()) {
                Some(value.trim().to_owned())
            } else {
                None
            }
        })
}
fn cpu_model() -> Option<String> {
    let mut text = String::new();
    File::open("/proc/cpuinfo")
        .ok()?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() > 1024 * 1024 {
        return None;
    }
    parse_cpu(&text)
}
pub fn scan() -> Hardware {
    let pci = run_bounded("lspci", &["-mm", "-nn"], false)
        .ok()
        .and_then(|s| parse_pci(&s).ok());
    let blocks = run_bounded(
        "lsblk",
        &[
            "--list",
            "--json",
            "--bytes",
            "-o",
            "TYPE,FSTYPE,SIZE,MOUNTPOINTS",
        ],
        false,
    )
    .ok()
    .and_then(|s| parse_lsblk(&s).ok());
    Hardware {
        evidence: vec![
            "/proc/cpuinfo:model name or Hardware".into(),
            "/sys/firmware/efi presence".into(),
            "lspci -mm -nn".into(),
            "lsblk --list --json --bytes -o TYPE,FSTYPE,SIZE,MOUNTPOINTS".into(),
        ],
        cpu_model: cpu_model(),
        uefi_visible: if Path::new("/sys/firmware/efi").is_dir() {
            Some(true)
        } else {
            None
        },
        pci_devices: pci,
        block_devices: blocks,
    }
}
