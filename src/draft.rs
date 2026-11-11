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
