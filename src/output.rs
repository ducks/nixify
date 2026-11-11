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
    out.push_str("\n## Manual migration requirements\n\n- Verify each proposed Nixpkgs attribute against the revision you intend to use.\n- Review all service configuration and data; enablement is not equivalent to a complete migration. Desktop hints do not establish your configured default; review hardware, networking, filesystems, and boot configuration separately.\n- Generate hardware configuration using NixOS installation tooling; this draft is not bootable or installable by itself.\n- Review foreign packages individually; they are not automatically mapped.\n");
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
    inventory.validate().map_err(io::Error::other)?;
    let decisions = mapping::decisions(inventory);
    // Never overwrite a previous output, including partially generated drafts.
    fs::create_dir(dir)?;
    create_file(&dir.join("packages.nix"), &packages_nix(&decisions))?;
    create_file(
        &dir.join("services.nix"),
        &services_nix(&service_mapping::decisions(inventory)),
    )?;
    create_file(&dir.join("desktop.nix"), &desktop_nix(inventory))?;
    create_file(
        &dir.join("migration-report.md"),
        &report(inventory, &decisions),
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
