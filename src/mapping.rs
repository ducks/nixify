use crate::inventory::{Inventory, Package};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Mapped,
    Review,
    Unmapped,
    Dependency,
}
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub package: String,
    pub status: Status,
    pub nixpkgs_attribute: Option<String>,
    pub reason: String,
}
// Deliberately small curated allowlist; these entries are not dynamically
// checked against a specific Nixpkgs revision during offline generation.
fn curated(name: &str) -> Option<&'static str> {
    match name {
        "git" => Some("git"),
        "firefox" => Some("firefox"),
        "neovim" => Some("neovim"),
        "ripgrep" => Some("ripgrep"),
        "curl" => Some("curl"),
        "htop" => Some("htop"),
        _ => None,
    }
}
pub fn decision(package: &Package) -> Decision {
    let (status, attr, reason) = if !package.explicitly_installed {
        (
            Status::Dependency,
            None,
            "Dependency-installed; not added as a direct system package.",
        )
    } else if package.foreign {
        (Status::Review, None, "Foreign package: provenance and equivalence require manual review; foreign does not mean AUR.")
    } else if package.name == "base-devel" {
        (
            Status::Review,
            None,
            "Package group has no single equivalent; choose development tools manually.",
        )
    } else if let Some(attr) = curated(&package.name) {
        (
            Status::Mapped,
            Some(attr.to_owned()),
            "Curated mapping; availability in the target Nixpkgs revision has not been validated.",
        )
    } else {
        (
            Status::Unmapped,
            None,
            "No curated mapping available; research and select a replacement manually.",
        )
    };
    Decision {
        package: package.name.clone(),
        status,
        nixpkgs_attribute: attr,
        reason: reason.into(),
    }
}
pub fn decisions(inventory: &Inventory) -> Vec<Decision> {
    let mut decisions: Vec<_> = inventory.packages.iter().map(decision).collect();
    decisions.sort_by(|a, b| a.package.cmp(&b.package));
    decisions
}
