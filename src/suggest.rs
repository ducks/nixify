use crate::{
    inventory::Inventory,
    manjaro,
    mapping::{self, Status},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Debug, Serialize)]
pub struct SuggestionReport {
    pub schema_version: u32,
    pub nixpkgs_checkout: String,
    pub checkout_declared_revision: Option<String>,
    pub target_system: String,
    pub suggestions: Vec<Suggestion>,
    pub warning: &'static str,
}
#[derive(Debug, Serialize)]
pub struct Suggestion {
    pub source_package: String,
    pub foreign: bool,
    pub candidate_attribute: String,
    pub attribute_exists: bool,
    pub classification: &'static str,
}

const EXPRESSION: &str = r#"{ nixpkgsPath, system, attributesJson }:
let
  pkgs = import (builtins.toPath nixpkgsPath) { inherit system; };
  attrs = builtins.fromJSON attributesJson;
in builtins.listToAttrs (map (name: { inherit name; value = builtins.hasAttr name pkgs; }) attrs)
"#;

fn candidates(inventory: &Inventory) -> Vec<String> {
    mapping::decisions(inventory)
        .into_iter()
        .filter(|d| matches!(d.status, Status::Unmapped | Status::Review))
        .map(|d| d.package)
        .collect()
}
fn declared_revision(checkout: &Path) -> Option<String> {
    let mut contents = String::new();
    fs::File::open(checkout.join(".git-revision"))
        .ok()?
        .take(129)
        .read_to_string(&mut contents)
        .ok()?;
    if contents.len() > 128 {
        return None;
    }
    crate::draft::normalize_revision(contents.trim()).ok()
}

pub fn report_from_results(
    inventory: &Inventory,
    checkout: &Path,
    revision: Option<String>,
    system: &str,
    results: BTreeMap<String, bool>,
) -> Result<SuggestionReport, String> {
    inventory.validate()?;
    let names = candidates(inventory);
    let expected: BTreeSet<_> = names.iter().collect();
    if results.len() != expected.len() || results.keys().any(|name| !expected.contains(name)) {
        return Err("Nixpkgs returned incomplete or unexpected attribute results".into());
    }
    let foreign: BTreeSet<&str> = inventory
        .packages
        .iter()
        .filter(|p| p.foreign)
        .map(|p| p.name.as_str())
        .collect();
    let suggestions = names
        .into_iter()
        .map(|name| Suggestion {
            foreign: foreign.contains(name.as_str()),
            attribute_exists: results[&name],
            candidate_attribute: name.clone(),
            source_package: name.clone(),
            classification: if results[&name] {
                "unreviewed_exact_name_match"
            } else {
                "no_exact_name_attribute"
            },
        })
        .collect();
    Ok(SuggestionReport { schema_version: 1, nixpkgs_checkout: checkout.display().to_string(), checkout_declared_revision: revision, target_system: system.into(), suggestions, warning: "An existing attribute with the same name is only a candidate: it may be unrelated, an alias, broken, unfree, or unavailable on the target platform. Foreign does not imply AUR. No mappings were accepted or generated. Checkout revision metadata is self-reported." })
}

pub fn evaluate(inventory: &Inventory, nixpkgs: &Path) -> Result<SuggestionReport, String> {
    inventory.validate()?;
    let checkout =
        fs::canonicalize(nixpkgs).map_err(|e| format!("cannot access Nixpkgs checkout: {e}"))?;
    let checkout_path = checkout
        .to_str()
        .ok_or("Nixpkgs checkout path is not valid UTF-8")?;
    let system = match inventory.source.architecture.as_str() {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        _ => {
            return Err("unsupported source architecture; choose a target platform manually".into())
        }
    };
    let names = candidates(inventory);
    let attrs = serde_json::to_string(&names).map_err(|e| e.to_string())?;
    if attrs.len() > 90_000 {
        return Err("too many package names for one safe evaluation; reduce the inventory".into());
    }
    let json = manjaro::run_bounded(
        "nix-instantiate",
        &[
            "--eval",
            "--strict",
            "--json",
            "-E",
            EXPRESSION,
            "--argstr",
            "nixpkgsPath",
            checkout_path,
            "--argstr",
            "system",
            system,
            "--argstr",
            "attributesJson",
            &attrs,
        ],
        false,
    )
    .map_err(|e| format!("candidate lookup failed: {e}"))?;
    let results =
        serde_json::from_str(&json).map_err(|e| format!("invalid candidate lookup output: {e}"))?;
    report_from_results(
        inventory,
        &checkout,
        declared_revision(&checkout),
        system,
        results,
    )
}
