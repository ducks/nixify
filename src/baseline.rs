use crate::{inventory::Inventory, manjaro};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema_version: u32,
    pub nixpkgs_path: String,
    pub nixpkgs_version: String,
    pub system: String,
    pub packages: Vec<String>,
}

// Evaluates a deliberately minimal NixOS configuration; never builds or activates.
// `pname` is a coarse name-based signal, not a package or behavior equivalence proof.
const EXPRESSION: &str = r#"{ nixpkgsPath }:
let
  root = builtins.toPath nixpkgsPath;
  pkgs = import root {};
  cfg = import (root + "/nixos/lib/eval-config.nix") {
    system = builtins.currentSystem;
    modules = [ ({ ... }: {
      system.stateVersion = "24.11";
      boot.loader.grub.devices = [ "nodev" ];
      fileSystems."/" = { device = "/dev/null"; fsType = "ext4"; };
    }) ];
  };
  names = builtins.filter (x: x != null) (map (pkg: pkg.pname or null) cfg.config.environment.systemPackages);
in {
  schema_version = 1;
  nixpkgs_path = nixpkgsPath;
  nixpkgs_version = pkgs.lib.version;
  system = builtins.currentSystem;
  packages = builtins.attrNames (builtins.listToAttrs (map (n: { name = n; value = true; }) names));
}"#;

pub fn evaluate(path: &Path) -> Result<Baseline, Box<dyn std::error::Error>> {
    let path = std::fs::canonicalize(path)?;
    let path = path.to_str().ok_or("Nixpkgs path must be valid UTF-8")?;
    let output = manjaro::run_bounded(
        "nix-instantiate",
        &[
            "--eval",
            "--strict",
            "--json",
            "-E",
            EXPRESSION,
            "--argstr",
            "nixpkgsPath",
            path,
        ],
        false,
    )?;
    let result: Baseline = serde_json::from_str(&output)?;
    validate(&result)?;
    Ok(result)
}
pub fn validate(baseline: &Baseline) -> Result<(), &'static str> {
    if baseline.schema_version != 1
        || baseline.nixpkgs_version.is_empty()
        || baseline.system.is_empty()
        || baseline.nixpkgs_path.is_empty()
    {
        return Err("invalid or unsupported NixOS baseline");
    }
    Ok(())
}
pub fn matches(inventory: &Inventory, baseline: &Baseline) -> Vec<String> {
    let names: BTreeSet<_> = baseline.packages.iter().map(String::as_str).collect();
    let mut result: Vec<_> = inventory
        .packages
        .iter()
        .filter(|p| p.explicitly_installed && names.contains(p.name.as_str()))
        .map(|p| p.name.clone())
        .collect();
    result.sort();
    result.dedup();
    result
}
