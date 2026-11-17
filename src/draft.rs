use std::{fs, path::Path};

pub const NIX_FILES: &[&str] = &[
    "flake.nix",
    "configuration.nix",
    "packages.nix",
    "services.nix",
    "desktop.nix",
];

pub fn default_revision() -> Result<String, String> {
    let lock: serde_json::Value = serde_json::from_str(include_str!("../flake.lock"))
        .map_err(|e| format!("invalid bundled lock file: {e}"))?;
    let revision = lock["nodes"]["nixpkgs"]["locked"]["rev"]
        .as_str()
        .ok_or("bundled Nixpkgs revision missing")?;
    normalize_revision(revision)
}
pub fn normalize_revision(revision: &str) -> Result<String, String> {
    if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Nixpkgs revision must be a full 40-character Git commit hash".into());
    }
    Ok(revision.to_ascii_lowercase())
}
pub fn flake(architecture: &str, revision: &str) -> Result<String, String> {
    let system = match architecture {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        other => {
            return Err(format!(
                "unsupported source architecture {other:?}; set the target platform manually"
            ))
        }
    };
    let revision = normalize_revision(revision)?;
    Ok(format!(
        r#"# Migration draft: review every generated module before use.
{{
  description = "nixify migration draft (not installable)";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/{revision}";
  outputs = {{ nixpkgs, ... }}: {{
    nixosConfigurations.nixify-draft = nixpkgs.lib.nixosSystem {{
      system = "{system}";
      modules = [ ./configuration.nix ];
    }};
  }};
}}
"#
    ))
}
pub fn configuration() -> &'static str {
    r#"# Incomplete migration draft: not a bootable or installable configuration.
# Add reviewed hardware configuration, filesystems, bootloader, users, networking,
# and system.stateVersion appropriate for the NixOS installation before use.
{ ... }:
{
  imports = [ ./packages.nix ./services.nix ./desktop.nix ];
}
"#
}

/// Performs syntax checks only. This intentionally does not evaluate a flake,
/// resolve remote inputs, build a derivation, or activate any configuration.
pub fn validate_syntax_with<F>(dir: &Path, mut parse: F) -> Result<(), String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    for name in NIX_FILES {
        let path = dir.join(name);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "expected a regular non-symlink file: {}",
                path.display()
            ));
        }
        if metadata.len() > 1024 * 1024 {
            return Err(format!("file exceeds 1 MiB: {}", path.display()));
        }
        parse(&path)?;
    }
    Ok(())
}
pub fn validate_syntax(dir: &Path) -> Result<(), String> {
    validate_syntax_with(dir, |path| {
        let absolute =
            fs::canonicalize(path).map_err(|e| format!("cannot access {}: {e}", path.display()))?;
        let text = absolute.to_str().ok_or("file path is not valid UTF-8")?;
        crate::manjaro::run_bounded("nix-instantiate", &["--parse", text], false)
            .map_err(|e| format!("syntax check failed for {}: {e}", path.display()))?;
        Ok(())
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum RevisionEvidence {
    DeclaredMatch,
    DeclaredMismatch(String),
    Unavailable,
}
#[derive(Debug)]
pub struct AttributeCheck {
    pub checkout: std::path::PathBuf,
    pub target_revision: String,
    pub revision_evidence: RevisionEvidence,
    pub attributes: std::collections::BTreeMap<String, bool>,
}

/// Accept only an unedited generated flake and packages module. Otherwise the
/// inventory cannot establish what the Nix files actually reference.
pub fn expected_attributes(
    dir: &Path,
    inventory: &crate::inventory::Inventory,
) -> Result<(String, Vec<String>), String> {
    inventory.validate()?;
    let flake_text = fs::read_to_string(dir.join("flake.nix"))
        .map_err(|e| format!("cannot read flake.nix: {e}"))?;
    let prefix = "  inputs.nixpkgs.url = \"github:NixOS/nixpkgs/";
    let revision = flake_text
        .lines()
        .find_map(|line| line.strip_prefix(prefix)?.strip_suffix("\";"))
        .ok_or("flake.nix is not the original generated flake; target pin cannot be established")?;
    let revision = normalize_revision(revision)?;
    if flake(&inventory.source.architecture, &revision)? != flake_text {
        return Err("flake.nix was edited; target pin cannot be verified by this check".into());
    }
    let review_path = dir.join("reviewed-mappings.json");
    let review = match fs::symlink_metadata(&review_path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            crate::mapping::read_reviewed(&review_path)?
        }
        Ok(_) => return Err("reviewed-mappings.json must be a regular non-symlink file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            crate::mapping::ReviewedMappings::empty()
        }
        Err(error) => return Err(format!("cannot inspect reviewed mappings: {error}")),
    };
    let decisions = crate::mapping::reviewed_decisions(inventory, &review, Some(&revision))?;
    let package_text = fs::read_to_string(dir.join("packages.nix"))
        .map_err(|e| format!("cannot read packages.nix: {e}"))?;
    // A snapshot keeps generated drafts verifiable after the bundled catalog changes.
    // It is data, not proof that the files have not been edited together.
    let snapshot_path = dir.join("package-decisions.json");
    let snapshot = match fs::symlink_metadata(&snapshot_path) {
        Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= 16 * 1024 * 1024 => {
            let bytes = fs::read(&snapshot_path)
                .map_err(|e| format!("cannot read package decisions: {e}"))?;
            let entries: Vec<crate::mapping::Decision> = serde_json::from_slice(&bytes)
                .map_err(|e| format!("invalid package decisions: {e}"))?;
            let packages: std::collections::BTreeMap<_, _> = inventory
                .packages
                .iter()
                .map(|p| (p.name.as_str(), p))
                .collect();
            let mut names = std::collections::BTreeSet::new();
            if entries.len() != packages.len()
                || entries.iter().any(|entry| {
                    let Some(package) = packages.get(entry.package.as_str()) else {
                        return true;
                    };
                    !names.insert(entry.package.as_str())
                        || match &entry.nixpkgs_attribute {
                            Some(attr) => {
                                !package.explicitly_installed
                                    || !crate::mapping::valid_attribute(attr)
                                    || !matches!(
                                        entry.status,
                                        crate::mapping::Status::Mapped
                                            | crate::mapping::Status::Inferred
                                            | crate::mapping::Status::Reviewed
                                    )
                            }
                            None => matches!(
                                entry.status,
                                crate::mapping::Status::Mapped
                                    | crate::mapping::Status::Inferred
                                    | crate::mapping::Status::Reviewed
                            ),
                        }
                })
            {
                return Err(
                    "package decisions do not match the inventory or contain unsafe attributes"
                        .into(),
                );
            }
            Some(entries)
        }
        Ok(_) => {
            return Err(
                "package-decisions.json must be a regular non-symlink file under 16 MiB".into(),
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot inspect package decisions: {error}")),
    };
    if let Some(entries) = &snapshot {
        if entries.iter().any(|d| d.covered_by.is_some()) {
            let read_module = |name: &str| -> Result<String, String> {
                let path = dir.join(name);
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|e| format!("cannot inspect {name}: {e}"))?;
                if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
                    return Err(format!(
                        "{name} must be a regular non-symlink file under 1 MiB"
                    ));
                }
                fs::read_to_string(path).map_err(|e| format!("cannot read {name}: {e}"))
            };
            if read_module("configuration.nix")? != configuration() {
                return Err("configuration.nix was edited; omitted packages need review".into());
            }
            let desktop_text = read_module("desktop.nix")?;
            let selected = [
                None,
                Some(crate::desktop::DesktopKind::Hyprland),
                Some(crate::desktop::DesktopKind::Xfce),
            ]
            .into_iter()
            .find(|selected| {
                desktop_text == crate::output::desktop_nix_with_selection(inventory, *selected)
            })
            .ok_or("desktop.nix was edited; omitted packages need review")?;
            let mut checked = entries.clone();
            crate::coverage::apply(inventory, &mut checked, &revision, selected);
            if entries
                .iter()
                .zip(&checked)
                .any(|(actual, expected)| actual.covered_by != expected.covered_by)
            {
                return Err(
                    "package coverage evidence does not match the inventory or target revision"
                        .into(),
                );
            }
            {
                let inspection_path = dir.join("configuration-inspections.json");
                let inspections: Vec<crate::config_inspection::Inspection> = match fs::symlink_metadata(&inspection_path) {
                    Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= 1024 * 1024 => {
                        let bytes = fs::read(&inspection_path).map_err(|e| format!("cannot read inspections: {e}"))?;
                        let values: Vec<crate::config_inspection::Inspection> = serde_json::from_slice(&bytes).map_err(|e| format!("invalid inspections: {e}"))?;
                        for value in &values { value.validate()?; }
                        values
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    _ => return Err("configuration-inspections.json must be a regular non-symlink file under 1 MiB".into()),
                };
                let expected = crate::output::services_nix_with_inspections(
                    &crate::service_mapping::decisions(inventory),
                    &inspections,
                );
                if read_module("services.nix")? != expected {
                    return Err(
                        "services.nix was edited; omitted service packages need review".into(),
                    );
                }
            }
        }
        if package_text == crate::output::packages_nix(entries) {
            let attrs = entries
                .iter()
                .filter(|d| d.covered_by.is_none())
                .filter_map(|d| d.nixpkgs_attribute.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            return Ok((revision, attrs));
        }
    }
    // Drafts before inferred mappings did not include catalog candidates.
    let mut pre_inference = decisions.clone();
    for decision in &mut pre_inference {
        if decision.status == crate::mapping::Status::Inferred {
            decision.nixpkgs_attribute = None;
        }
    }
    // Drafts predating decision snapshots used the original bundled catalog.
    // Keep those drafts checkable when new curated entries are added.
    let old_db = crate::mapping::parse_database(include_str!("../mappings/manjaro-v1.json"))?;
    let old_names: std::collections::BTreeSet<_> = old_db
        .entries
        .iter()
        .map(|e| e.source_package.as_str())
        .collect();
    let mut prior_curated = pre_inference.clone();
    for decision in &mut prior_curated {
        if decision.status == crate::mapping::Status::Mapped
            && !old_names.contains(decision.package.as_str())
        {
            decision.nixpkgs_attribute = None;
        }
    }
    // Drafts created before the curated list expanded must stay verifiable.
    // Reconstruct the original six-entry mapping rather than interpreting Nix.
    let mut original_decisions = crate::mapping::decisions(inventory);
    for decision in &mut original_decisions {
        if decision.status == crate::mapping::Status::Inferred
            || (decision.status == crate::mapping::Status::Mapped
                && !["git", "firefox", "neovim", "ripgrep", "curl", "htop"]
                    .contains(&decision.package.as_str()))
        {
            decision.nixpkgs_attribute = None;
        }
    }
    let prior_six = review.mappings.is_empty()
        && (package_text == crate::output::legacy_packages_nix(&original_decisions)
            || package_text == crate::output::packages_nix(&original_decisions));
    let prior_catalog =
        snapshot.is_none() && package_text == crate::output::packages_nix(&pre_inference);
    let prior_curated_match =
        snapshot.is_none() && package_text == crate::output::packages_nix(&prior_curated);
    if !prior_six && !prior_catalog && !prior_curated_match {
        return Err(
            "packages.nix or inventory was edited; cannot reliably enumerate referenced attributes"
                .into(),
        );
    }
    let checked_decisions = if prior_six {
        original_decisions
    } else if prior_catalog {
        pre_inference
    } else {
        prior_curated
    };
    let attrs = checked_decisions
        .into_iter()
        .filter_map(|d| d.nixpkgs_attribute)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok((revision, attrs))
}

fn revision_evidence(checkout: &Path, target: &str) -> RevisionEvidence {
    use std::io::Read;
    let mut text = String::new();
    let ok = fs::File::open(checkout.join(".git-revision"))
        .and_then(|f| f.take(129).read_to_string(&mut text))
        .is_ok();
    if !ok || text.len() > 128 {
        return RevisionEvidence::Unavailable;
    }
    match normalize_revision(text.trim()) {
        Ok(revision) if revision == target => RevisionEvidence::DeclaredMatch,
        Ok(revision) => RevisionEvidence::DeclaredMismatch(revision),
        Err(_) => RevisionEvidence::Unavailable,
    }
}

const ATTRIBUTE_EXPRESSION: &str = r#"{ nixpkgsPath, system, attributesJson }:
let
  pkgs = import (builtins.toPath nixpkgsPath) { inherit system; };
  attrs = builtins.fromJSON attributesJson;
in builtins.listToAttrs (map (name: { inherit name; value = pkgs.lib.hasAttrByPath (pkgs.lib.splitString "." name) pkgs; }) attrs)
"#;

pub fn check_attributes(
    dir: &Path,
    inventory: &crate::inventory::Inventory,
    nixpkgs: &Path,
) -> Result<AttributeCheck, String> {
    let (target_revision, attrs) = expected_attributes(dir, inventory)?;
    let checkout =
        fs::canonicalize(nixpkgs).map_err(|e| format!("cannot access Nixpkgs checkout: {e}"))?;
    let nixpkgs_path = checkout
        .to_str()
        .ok_or("Nixpkgs path must be valid UTF-8")?;
    let system = match inventory.source.architecture.as_str() {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        _ => return Err("unsupported target architecture".into()),
    };
    let attrs_json = serde_json::to_string(&attrs).map_err(|e| e.to_string())?;
    let data = crate::manjaro::run_bounded(
        "nix-instantiate",
        &[
            "--eval",
            "--strict",
            "--json",
            "-E",
            ATTRIBUTE_EXPRESSION,
            "--argstr",
            "nixpkgsPath",
            nixpkgs_path,
            "--argstr",
            "system",
            system,
            "--argstr",
            "attributesJson",
            &attrs_json,
        ],
        false,
    )
    .map_err(|e| format!("attribute check failed: {e}"))?;
    let attributes: std::collections::BTreeMap<String, bool> = serde_json::from_str(&data)
        .map_err(|e| format!("invalid attribute evaluation result: {e}"))?;
    if attributes.len() != attrs.len() || attrs.iter().any(|a| !attributes.contains_key(a)) {
        return Err("attribute evaluation returned incomplete results".into());
    }
    let revision_evidence = revision_evidence(&checkout, &target_revision);
    Ok(AttributeCheck {
        checkout,
        target_revision,
        revision_evidence,
        attributes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revision_metadata_is_only_a_declared_match() {
        let dir = tempfile::tempdir().unwrap();
        let target = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            revision_evidence(dir.path(), target),
            RevisionEvidence::Unavailable
        );
        fs::write(dir.path().join(".git-revision"), format!("{target}\n")).unwrap();
        assert_eq!(
            revision_evidence(dir.path(), target),
            RevisionEvidence::DeclaredMatch
        );
        fs::write(
            dir.path().join(".git-revision"),
            "ffffffffffffffffffffffffffffffffffffffff",
        )
        .unwrap();
        assert!(matches!(
            revision_evidence(dir.path(), target),
            RevisionEvidence::DeclaredMismatch(_)
        ));
        fs::write(dir.path().join(".git-revision"), "untrusted text").unwrap();
        assert_eq!(
            revision_evidence(dir.path(), target),
            RevisionEvidence::Unavailable
        );
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleEvaluation {
    pub state_version: String,
    pub system_package_count: usize,
    pub failed_assertions: Vec<String>,
}

// This evaluates the supplied project; users must explicitly opt in and trust it.
// The dummy options allow evaluating an incomplete draft but are never written
// to the project and do not describe the source machine's actual hardware.
const MODULE_EXPRESSION: &str = r#"{ nixpkgsPath, projectPath, system }:
let
  nixpkgs = builtins.toPath nixpkgsPath;
  project = builtins.toPath projectPath;
  evaluated = import (nixpkgs + "/nixos/lib/eval-config.nix") {
    inherit system;
    modules = [ (project + "/configuration.nix") ({ lib, ... }: {
      system.stateVersion = lib.mkDefault "24.11";
      boot.loader.grub.devices = lib.mkDefault [ "nodev" ];
      fileSystems."/" = { device = lib.mkDefault "/dev/null"; fsType = lib.mkDefault "ext4"; };
    }) ];
  };
  cfg = evaluated.config;
  packageNames = map (pkg: pkg.name) cfg.environment.systemPackages;
  failed = map (assertion: assertion.message) (builtins.filter (assertion: !assertion.assertion) cfg.assertions);
in builtins.deepSeq packageNames {
  state_version = cfg.system.stateVersion;
  system_package_count = builtins.length packageNames;
  failed_assertions = failed;
}
"#;

pub fn evaluate_modules(
    dir: &Path,
    nixpkgs: &Path,
    inventory: &crate::inventory::Inventory,
) -> Result<ModuleEvaluation, String> {
    evaluate_modules_with(dir, nixpkgs, inventory, |args| {
        crate::manjaro::run_bounded("nix-instantiate", args, false).map_err(|e| e.to_string())
    })
}

pub fn evaluate_modules_with<F>(
    dir: &Path,
    nixpkgs: &Path,
    inventory: &crate::inventory::Inventory,
    run: F,
) -> Result<ModuleEvaluation, String>
where
    F: FnOnce(&[&str]) -> Result<String, String>,
{
    inventory.validate()?;
    let project =
        fs::canonicalize(dir).map_err(|e| format!("cannot access project directory: {e}"))?;
    let checkout =
        fs::canonicalize(nixpkgs).map_err(|e| format!("cannot access Nixpkgs checkout: {e}"))?;
    let project_path = project.to_str().ok_or("project path must be valid UTF-8")?;
    let nixpkgs_path = checkout
        .to_str()
        .ok_or("Nixpkgs path must be valid UTF-8")?;
    let system = match inventory.source.architecture.as_str() {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        _ => return Err("unsupported target architecture".into()),
    };
    let json = run(&[
        "--eval",
        "--strict",
        "--json",
        "-E",
        MODULE_EXPRESSION,
        "--argstr",
        "nixpkgsPath",
        nixpkgs_path,
        "--argstr",
        "projectPath",
        project_path,
        "--argstr",
        "system",
        system,
    ])
    .map_err(|e| format!("module evaluation failed: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("invalid module evaluation result: {e}"))
}
