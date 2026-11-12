use crate::{
    desktop,
    inventory::Inventory,
    mapping::{self, Decision, Status},
    service_mapping::{self, ServiceDecision},
};
use std::{
    collections::BTreeSet,
    fmt::Write,
    fs,
    io::{self, Write as _},
    path::Path,
};

pub fn packages_nix(decisions: &[Decision]) -> String {
    let attrs: BTreeSet<_> = decisions
        .iter()
        .filter_map(|d| d.nixpkgs_attribute.as_deref())
        .collect();
    let mut out = String::from("# Migration draft. Curated mappings are not yet validated against a Nixpkgs revision.\n{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [\n");
    for attr in attrs {
        let _ = writeln!(out, "    {attr}");
    }
    out.push_str("  ];\n}\n");
    out
}
pub fn services_nix(decisions: &[ServiceDecision]) -> String {
    let options: BTreeSet<_> = decisions.iter().filter_map(|d| d.nixos_option).collect();
    let mut out = String::from("# Migration draft: enablement only; review service configuration and target options.\n{ ... }:\n{\n");
    for option in options {
        let _ = writeln!(out, "  {option} = true;");
    }
    out.push_str("}\n");
    out
}

pub fn desktop_nix(inventory: &Inventory) -> String {
    let mut out = String::from("# Suggestions only: confirm the intended desktop and target NixOS options before enabling.\n{ ... }:\n{\n");
    let options: BTreeSet<_> = inventory
        .desktop
        .as_ref()
        .into_iter()
        .flat_map(|desktop| desktop.candidates.iter())
        .filter_map(|candidate| desktop::suggested_option(candidate.kind))
        .collect();
    for option in options {
        let _ = writeln!(out, "  # {option} = true;");
    }
    out.push_str("}\n");
    out
}

fn markdown_text(input: &str) -> String {
    let mut result = String::new();
    for c in input.chars() {
        if "\\`*_{}[]()<>#+-.!|".contains(c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

pub fn report(inventory: &Inventory, decisions: &[Decision]) -> String {
    let count = |s: Status| decisions.iter().filter(|d| d.status == s).count();
    let explicit = inventory
        .packages
        .iter()
        .filter(|p| p.explicitly_installed)
        .count();
    let mut out = format!("# nixify migration report\n\nSource: Manjaro Linux ({})\n\n## Packages\n\n- Explicitly installed: {explicit}\n- Curated mappings (not revision-validated): {}\n- Require review: {}\n- No mapping: {}\n- Dependency-installed, not selected: {}\n\n", inventory.source.architecture, count(Status::Mapped), count(Status::Review), count(Status::Unmapped), count(Status::Dependency));
    out.push_str("## Package decisions\n\n");
    for d in decisions.iter().filter(|d| d.status != Status::Dependency) {
        let _ = writeln!(
            out,
            "- `{}`: {}{}",
            d.package,
            d.reason,
            d.nixpkgs_attribute
                .as_ref()
                .map(|a| format!(" Proposed attribute: `{a}`."))
                .unwrap_or_default()
        );
    }
    out.push_str("\n## Services\n\n");
    if inventory.service_scopes.is_empty() {
        out.push_str(
            "Service discovery was not included in this inventory; service state is unknown.\n",
        );
    }
    for scope in &inventory.service_scopes {
        let _ = writeln!(
            out,
            "- {:?} scope: unit files {}, running-state {}.",
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
        );
    }
    for d in service_mapping::decisions(inventory) {
        let _ = writeln!(
            out,
            "- {:?} `{}`: enablement {:?}; running {:?}. {}{}",
            d.scope,
            d.service,
            d.unit_file_state,
            d.running,
            d.reason,
            d.nixos_option
                .map(|option| format!(" Proposed option: `{option}`."))
                .unwrap_or_default()
        );
    }
    out.push_str("\n## Desktop and display manager\n\n");
    if let Some(desktop) = &inventory.desktop {
        let _ = writeln!(out, "Current process session type: {:?}. Session environment is a hint, not proof of a configured default.", desktop.current_process_session_type);
        let mut candidates = desktop.candidates.iter().collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.kind);
        if candidates.is_empty() {
            out.push_str("No recognized desktop evidence was found; the configured desktop remains unknown.\n");
        }
        for candidate in candidates {
            let _ = writeln!(out, "- {:?}: installed package evidence {:?}; X11 session file {:?}; Wayland session file {:?}; current process desktop hint {}.", candidate.kind, candidate.installed_packages, candidate.x11_session_file, candidate.wayland_session_file, candidate.current_process_session_hint);
        }
    } else {
        out.push_str("Desktop discovery was not included in this inventory.\n");
    }
    let managers = desktop::enabled_display_managers(inventory);
    if managers.is_empty() {
        out.push_str("Enabled display manager is unknown or not among the recognized services.\n");
    } else {
        let _ = writeln!(out, "Recognized enabled display-manager services: {:?}; confirm their configuration manually.", managers);
    }
    out.push_str("Desktop module options in desktop.nix are commented suggestions only. Verify the target NixOS revision and select the intended desktop and display manager.\n");
    out.push_str("\n## Hardware and filesystems (planning evidence)\n\n");
    if let Some(hardware) = &inventory.hardware {
        let _ = writeln!(
            out,
            "CPU model: {}.",
            hardware
                .cpu_model
                .as_deref()
                .map(markdown_text)
                .unwrap_or_else(|| "unknown".into())
        );
        out.push_str(if hardware.uefi_visible == Some(true) {
            "UEFI firmware interface is visible; verify the intended boot setup.\n"
        } else {
            "Boot mode is unknown; absence of UEFI evidence does not prove BIOS.\n"
        });
        out.push_str("Sources: /proc/cpuinfo, /sys/firmware/efi, lspci and lsblk; unavailable evidence is unknown.\n");
        match &hardware.pci_devices {
            Some(devices) => {
                for device in devices {
                    let _ = writeln!(
                        out,
                        "- {}: {} — {}",
                        device.category,
                        markdown_text(&device.vendor),
                        markdown_text(&device.model)
                    );
                }
            }
            None => out.push_str("PCI device evidence unavailable.\n"),
        }
        match &hardware.block_devices {
            Some(devices) => {
                for device in devices {
                    let _ = writeln!(
                        out,
                        "- Block device type `{}`: filesystem {:?}, {} bytes, system mounts {:?}.",
                        device.kind, device.filesystem, device.size_bytes, device.system_mounts
                    );
                }
            }
            None => out.push_str("Block device evidence unavailable.\n"),
        }
        out.push_str("Device identifiers, serial numbers, UUIDs and non-system mount paths are omitted; no disk or filesystem configuration is generated.\n");
    } else {
        out.push_str("Hardware discovery was not included in this inventory.\n");
    }
    out.push_str("\n## Networking (planning evidence)\n\n");
    if let Some(network) = &inventory.networking {
        match &network.interfaces {
            Some(i) => {
                let _ = writeln!(out, "Interface categories: {} loopback, {} wireless, {} Ethernet or virtual, {} other. Names and addresses are omitted.", i.loopback, i.wireless, i.ethernet_or_virtual, i.other);
            }
            None => out.push_str("Network interface evidence is unavailable.\n"),
        }
        match &network.dns {
            Some(d) => {
                let _ = writeln!(out, "Resolver file: {} loopback, {} other and {} unrecognized nameserver entries. Addresses and search domains are omitted; actual DNS in use may differ.", d.loopback_nameservers, d.other_nameservers, d.unrecognized_nameservers);
            }
            None => {
                out.push_str("Resolver file evidence is unavailable or could not be read safely.\n")
            }
        }
        let _ = writeln!(out, "Firewall-related installed packages: {:?}; installed does not establish active firewall rules.", network.firewall_packages);
    } else {
        out.push_str("Network discovery was not included in this inventory.\n");
    }
    if let Some(scope) = inventory
        .service_scopes
        .iter()
        .find(|s| s.scope == crate::inventory::Scope::System)
    {
        for name in ["NetworkManager.service", "systemd-networkd.service"] {
            if let Some(service) = scope.services.iter().find(|s| s.name == name) {
                let _ = writeln!(
                    out,
                    "- {}: unit-file state {:?}; running state {:?}.",
                    name, service.unit_file_state, service.running
                );
            } else {
                let _ = writeln!(out, "- {}: not observed; state is unknown.", name);
            }
        }
    } else {
        out.push_str("Network manager service state is unknown.\n");
    }
    out.push_str("Hostname, interface configuration, DNS addresses, VPNs and effective firewall rules require manual review.\n");
    out.push_str("\n## Manual migration requirements\n\n- Verify each proposed Nixpkgs attribute against the revision you intend to use.\n- Review all service configuration and data; enablement is not equivalent to a complete migration. Desktop hints do not establish your configured default; verify hardware, networking, filesystems, and boot configuration separately.\n- Generate hardware configuration using NixOS installation tooling; this draft is not bootable or installable by itself.\n- Review foreign packages individually; they are not automatically mapped.\n");
    out
}
fn create_file(path: &Path, data: &str) -> io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(data.as_bytes())
}
pub fn generate(inventory: &Inventory, dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let revision = crate::draft::default_revision().map_err(io::Error::other)?;
    generate_with_revision(inventory, dir, &revision)
}
pub fn generate_with_revision(
    inventory: &Inventory,
    dir: &Path,
    revision: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    inventory.validate().map_err(io::Error::other)?;
    let flake =
        crate::draft::flake(&inventory.source.architecture, revision).map_err(io::Error::other)?;
    let revision = crate::draft::normalize_revision(revision).map_err(io::Error::other)?;
    let decisions = mapping::decisions(inventory);
    // Never overwrite a previous output, including partially generated drafts.
    fs::create_dir(dir)?;
    create_file(&dir.join("flake.nix"), &flake)?;
    create_file(
        &dir.join("configuration.nix"),
        crate::draft::configuration(),
    )?;
    create_file(&dir.join("packages.nix"), &packages_nix(&decisions))?;
    create_file(
        &dir.join("services.nix"),
        &services_nix(&service_mapping::decisions(inventory)),
    )?;
    create_file(&dir.join("desktop.nix"), &desktop_nix(inventory))?;
    create_file(
        &dir.join("migration-report.md"),
        &format!("{}\n## Target draft\n\n- Nixpkgs revision: `{}` (not yet resolved or validated).\n- No flake.lock was generated.\n- Supply reviewed hardware, boot, filesystems, users, networking and system.stateVersion before use.\n", report(inventory, &decisions), revision),
    )?;
    let mut sorted = inventory.clone();
    sorted.packages.sort_by(|a, b| a.name.cmp(&b.name));
    sorted.service_scopes.sort_by_key(|s| s.scope);
    for scope in &mut sorted.service_scopes {
        scope.services.sort_by(|a, b| a.name.cmp(&b.name));
    }
    if let Some(desktop) = &mut sorted.desktop {
        desktop.candidates.sort_by_key(|candidate| candidate.kind);
        for candidate in &mut desktop.candidates {
            candidate.installed_packages.sort();
        }
    }
    create_file(
        &dir.join("inventory.json"),
        &(serde_json::to_string_pretty(&sorted)? + "\n"),
    )?;
    Ok(())
}
