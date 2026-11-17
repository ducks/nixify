//! Conservative, revision-scoped evidence for packages already provided by
//! generated NixOS modules. These rules do not establish source equivalence.
use crate::{
    desktop::DesktopKind,
    inventory::Inventory,
    mapping::{Decision, Status},
    service_mapping,
};
use serde::{Deserialize, Serialize};

const VERIFIED_REVISIONS: &[&str] = &[
    "eeeb90a1dd3c9bea3afdbc76fd34d0fb2a727c7a",
    "9fbb54b33e91ee4ca368e35a78e0613c720600b3",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub provider: String,
    pub nixpkgs_revision: String,
}

/// Applies only when both the source name and mapped attribute are exact. A
/// user-reviewed mapping always wins. No rule is applied to foreign packages.
pub fn apply(
    inventory: &Inventory,
    decisions: &mut [Decision],
    revision: &str,
    desktop: Option<DesktopKind>,
) {
    for decision in decisions.iter_mut() {
        decision.covered_by = None;
    }
    if inventory.source.architecture != "x86_64" || !VERIFIED_REVISIONS.contains(&revision) {
        return;
    }
    let services = service_mapping::decisions(inventory);
    for decision in decisions {
        if decision.status == Status::Reviewed
            || !matches!(decision.status, Status::Mapped | Status::Inferred)
        {
            continue;
        }
        let Some(package) = inventory
            .packages
            .iter()
            .find(|p| p.name == decision.package)
        else {
            continue;
        };
        if package.foreign
            || decision.nixpkgs_attribute.as_deref() != Some(decision.package.as_str())
        {
            continue;
        }
        let provider = match decision.package.as_str() {
            // These are in requiredPackages/corePackageNames or the defaultPackages
            // list in nixos/modules/config/system-path.nix at both revisions.
            "bzip2" | "diffutils" | "findutils" | "gawk" | "gzip" | "less" | "which" => {
                Some("NixOS required system packages")
            }
            "perl" | "rsync" => Some("NixOS default system packages"),
            "networkmanager"
                if services
                    .iter()
                    .any(|s| s.nixos_option == Some("networking.networkmanager.enable")) =>
            {
                Some("networking.networkmanager.enable")
            }
            "docker"
                if services
                    .iter()
                    .any(|s| s.nixos_option == Some("virtualisation.docker.enable")) =>
            {
                Some("virtualisation.docker.enable")
            }
            "hyprland" if desktop == Some(DesktopKind::Hyprland) => {
                Some("programs.hyprland.enable")
            }
            _ => None,
        };
        if let Some(provider) = provider {
            decision.covered_by = Some(Coverage {
                provider: provider.into(),
                nixpkgs_revision: revision.into(),
            });
        }
    }
}
