//! Installed pacman group membership is a review hint, not evidence of intent.
use crate::inventory::{valid_package_name, Inventory, PackageGroup};
use std::collections::{BTreeMap, BTreeSet};

pub fn parse(output: &str, inventory: &Inventory) -> Result<Vec<PackageGroup>, String> {
    let installed: BTreeSet<_> = inventory.packages.iter().map(|p| p.name.as_str()).collect();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2
            || !valid_package_name(fields[0])
            || !valid_package_name(fields[1])
            || !installed.contains(fields[1])
        {
            return Err(
                "invalid or inconsistent pacman -Qg output; package database may have changed"
                    .into(),
            );
        }
        if !groups
            .entry(fields[0].into())
            .or_default()
            .insert(fields[1].into())
        {
            return Err("duplicate pacman package group membership".into());
        }
    }
    Ok(groups
        .into_iter()
        .map(|(name, packages)| PackageGroup {
            name,
            packages: packages.into_iter().collect(),
        })
        .collect())
}
