use crate::{
    inventory::Inventory,
    mapping::{self, Decision, Status},
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
    out.push_str("\n## Manual migration requirements\n\n- Verify each proposed Nixpkgs attribute against the revision you intend to use.\n- Review services, desktop, hardware, networking, filesystems, and boot configuration; this version did not scan them.\n- Generate hardware configuration using NixOS installation tooling; this draft is not bootable or installable by itself.\n- Review foreign packages individually; they are not automatically mapped.\n");
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
        &dir.join("migration-report.md"),
        &report(inventory, &decisions),
    )?;
    let mut sorted = inventory.clone();
    sorted.packages.sort_by(|a, b| a.name.cmp(&b.name));
    create_file(
        &dir.join("inventory.json"),
        &(serde_json::to_string_pretty(&sorted)? + "\n"),
    )?;
    Ok(())
}
