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
    let is_legacy = review.mappings.is_empty()
        && package_text == crate::output::legacy_packages_nix(&decisions);
    if package_text != crate::output::packages_nix(&decisions) && !is_legacy {
        return Err(
            "packages.nix or inventory was edited; cannot reliably enumerate referenced attributes"
                .into(),
        );
    }
    let attrs = decisions
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
in builtins.listToAttrs (map (name: { inherit name; value = builtins.hasAttr name pkgs; }) attrs)
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
