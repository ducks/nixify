//! Explicit review of a small set of application roles. Never infer disuse.
use crate::{inventory::Inventory, mapping::Decision};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub schema_version: u32,
    pub groups: Vec<Group>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub role: String,
    pub packages: Vec<Choice>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub package: String,
    pub keep: bool,
}

fn role(name: &str) -> Option<&'static str> {
    match name {
        "firefox" | "chromium" | "librewolf" | "brave-browser" | "vivaldi" | "google-chrome" => {
            Some("Web browsers")
        }
        "neovim" | "vim" | "helix" | "emacs" | "code" | "code-oss" => Some("Code editors"),
        "alacritty" | "kitty" | "wezterm" | "gnome-terminal" | "xfce4-terminal" | "konsole" => {
            Some("Terminal emulators")
        }
        "thunar" | "dolphin" | "nautilus" | "pcmanfm" | "pcmanfm-qt" => Some("File managers"),
        _ => None,
    }
}

/// Only present roles with multiple explicit source packages. Keeping all is
/// the safe default: categories do not establish intent or equivalence.
pub fn propose(inventory: &Inventory) -> Selection {
    let mut groups: BTreeMap<&str, Vec<Choice>> = BTreeMap::new();
    for package in inventory.packages.iter().filter(|p| p.explicitly_installed) {
        if let Some(role) = role(&package.name) {
            groups.entry(role).or_default().push(Choice {
                package: package.name.clone(),
                keep: true,
            });
        }
    }
    let groups = groups
        .into_iter()
        .filter_map(|(role, mut packages)| {
            if packages.len() < 2 {
                return None;
            }
            packages.sort_by(|a, b| a.package.cmp(&b.package));
            Some(Group {
                role: role.into(),
                packages,
            })
        })
        .collect();
    Selection {
        schema_version: 1,
        groups,
    }
}

pub fn validate(selection: &Selection, inventory: &Inventory) -> Result<(), String> {
    if selection.schema_version != 1 {
        return Err("unsupported selection schema version".into());
    }
    let expected = propose(inventory);
    let canonical = |s: &Selection| -> Result<BTreeMap<String, BTreeSet<String>>, String> {
        let mut groups = BTreeMap::new();
        for group in &s.groups {
            let mut members = BTreeSet::new();
            for p in &group.packages {
                if !members.insert(p.package.clone()) {
                    return Err("duplicate package selection".into());
                }
            }
            if groups.insert(group.role.clone(), members).is_some() {
                return Err("duplicate selection group".into());
            }
        }
        Ok(groups)
    };
    if canonical(selection)? != canonical(&expected)? {
        return Err(
            "selection does not match the inventory and current review groups; create a new review"
                .into(),
        );
    }
    Ok(())
}

pub fn apply(
    selection: &Selection,
    inventory: &Inventory,
    decisions: &mut [Decision],
) -> Result<(), String> {
    validate(selection, inventory)?;
    let excluded: BTreeSet<_> = selection
        .groups
        .iter()
        .flat_map(|g| &g.packages)
        .filter(|p| !p.keep)
        .map(|p| p.package.as_str())
        .collect();
    for decision in decisions {
        decision.excluded_by_selection = excluded.contains(decision.package.as_str());
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Selection, String> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|e| format!("cannot inspect selection: {e}"))?;
    if !meta.file_type().is_file() || meta.len() > 1024 * 1024 {
        return Err("selection must be a regular non-symlink file under 1 MiB".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("selection exceeds 1 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid selection: {e}"))
}

/// Enter means keep all; numeric lists or 'none' are explicit choices.
pub fn review<R: io::BufRead, W: Write>(
    selection: &mut Selection,
    input: &mut R,
    output: &mut W,
) -> Result<(), String> {
    for group in &mut selection.groups {
        writeln!(output, "\n{} (all selected by default):", group.role)
            .map_err(|e| e.to_string())?;
        for (index, item) in group.packages.iter().enumerate() {
            writeln!(output, "  {}. {}", index + 1, item.package).map_err(|e| e.to_string())?;
        }
        loop {
            write!(
                output,
                "Keep numbers separated by commas, 'none', or Enter for all: "
            )
            .and_then(|_| output.flush())
            .map_err(|e| e.to_string())?;
            let mut line = String::new();
            if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("review cancelled before completion".into());
            }
            let value = line.trim();
            if value.is_empty() {
                break;
            }
            if value.eq_ignore_ascii_case("none") {
                for item in &mut group.packages {
                    item.keep = false;
                }
                break;
            }
            let parsed: Result<BTreeSet<usize>, _> = value
                .split(',')
                .map(|s| s.trim().parse::<usize>())
                .collect();
            if let Ok(numbers) = parsed {
                if !numbers.is_empty()
                    && numbers
                        .iter()
                        .all(|n| *n >= 1 && *n <= group.packages.len())
                {
                    for (i, item) in group.packages.iter_mut().enumerate() {
                        item.keep = numbers.contains(&(i + 1));
                    }
                    break;
                }
            }
            writeln!(output, "Invalid selection; try again.").map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
