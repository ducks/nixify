use crate::inventory::{Inventory, Package};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Mapped,
    Reviewed,
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
        "alacritty" => Some("alacritty"),
        "bat" => Some("bat"),
        "btop" => Some("btop"),
        "fd" => Some("fd"),
        "ffmpeg" => Some("ffmpeg"),
        "fish" => Some("fish"),
        "fzf" => Some("fzf"),
        "gcc" => Some("gcc"),
        "jq" => Some("jq"),
        "kitty" => Some("kitty"),
        "less" => Some("less"),
        "make" => Some("gnumake"),
        "nano" => Some("nano"),
        "nmap" => Some("nmap"),
        "openssh" => Some("openssh"),
        "rsync" => Some("rsync"),
        "shellcheck" => Some("shellcheck"),
        "tmux" => Some("tmux"),
        "tree" => Some("tree"),
        "unzip" => Some("unzip"),
        "vlc" => Some("vlc"),
        "wget" => Some("wget"),
        "which" => Some("which"),
        "xclip" => Some("xclip"),
        "zsh" => Some("zsh"),
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
            "Curated package mapping; target-revision availability, configuration and behavior are not yet verified.",
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedMappings {
    pub schema_version: u32,
    /// Optional user-declared revision; matching it does not authenticate content.
    pub nixpkgs_revision: Option<String>,
    pub mappings: Vec<ReviewedMapping>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedMapping {
    pub source_package: String,
    /// A single top-level Nixpkgs attribute, never an expression or attribute path.
    pub nixpkgs_attribute: String,
}
impl ReviewedMappings {
    pub fn empty() -> Self {
        Self {
            schema_version: 1,
            nixpkgs_revision: None,
            mappings: Vec::new(),
        }
    }
}
pub fn read_reviewed(path: &Path) -> Result<ReviewedMappings, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("cannot open reviewed mappings: {e}"))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read reviewed mappings: {e}"))?;
    if bytes.len() > 1024 * 1024 {
        return Err("reviewed mappings exceed 1 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid reviewed mappings: {e}"))
}
fn valid_attribute(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
}
pub fn reviewed_decisions(
    inventory: &Inventory,
    review: &ReviewedMappings,
    target_revision: Option<&str>,
) -> Result<Vec<Decision>, String> {
    inventory.validate()?;
    if review.schema_version != 1 {
        return Err("unsupported reviewed mappings schema version".into());
    }
    if let Some(revision) = &review.nixpkgs_revision {
        let normalized = crate::draft::normalize_revision(revision)?;
        if target_revision.is_some_and(|target| target != normalized) {
            return Err(
                "reviewed mappings declare a different Nixpkgs revision from the draft".into(),
            );
        }
    }
    let packages: BTreeMap<_, _> = inventory
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let mut overrides = BTreeMap::new();
    for item in &review.mappings {
        if !valid_attribute(&item.nixpkgs_attribute) {
            return Err(format!(
                "invalid top-level Nixpkgs attribute: {:?}",
                item.nixpkgs_attribute
            ));
        }
        let package = packages
            .get(item.source_package.as_str())
            .ok_or_else(|| format!("reviewed package not found: {:?}", item.source_package))?;
        if !package.explicitly_installed {
            return Err(format!(
                "reviewed package was not explicitly installed: {:?}",
                item.source_package
            ));
        }
        if overrides
            .insert(
                item.source_package.as_str(),
                item.nixpkgs_attribute.as_str(),
            )
            .is_some()
        {
            return Err(format!(
                "duplicate reviewed package: {:?}",
                item.source_package
            ));
        }
    }
    let mut result = decisions(inventory);
    for decision in &mut result {
        if let Some(attr) = overrides.get(decision.package.as_str()) {
            decision.status = Status::Reviewed;
            decision.nixpkgs_attribute = Some((*attr).to_string());
            decision.reason = "User-reviewed mapping; Nixpkgs availability and behavioral equivalence are not automatically verified.".into();
        }
    }
    Ok(result)
}
