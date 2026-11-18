use crate::inventory::{Inventory, Scope};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopKind {
    Plasma,
    Gnome,
    Xfce,
    Hyprland,
    Sway,
    I3,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionType {
    X11,
    Wayland,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub kind: DesktopKind,
    pub installed_packages: Vec<String>,
    pub x11_session_file: Option<bool>,
    pub wayland_session_file: Option<bool>,
    /// A recognized XDG_CURRENT_DESKTOP value for this process only.
    pub current_process_session_hint: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopInventory {
    pub candidates: Vec<Candidate>,
    /// Recognized XDG_SESSION_TYPE for this process only.
    pub current_process_session_type: Option<SessionType>,
}

fn ensure(candidates: &mut BTreeMap<DesktopKind, Candidate>, kind: DesktopKind) -> &mut Candidate {
    candidates.entry(kind).or_insert_with(|| Candidate {
        kind,
        installed_packages: Vec::new(),
        x11_session_file: None,
        wayland_session_file: None,
        current_process_session_hint: false,
    })
}
fn package_kind(name: &str) -> Option<DesktopKind> {
    match name {
        "plasma-desktop" | "plasma-workspace" => Some(DesktopKind::Plasma),
        "gnome-shell" => Some(DesktopKind::Gnome),
        "xfce4-session" => Some(DesktopKind::Xfce),
        "hyprland" => Some(DesktopKind::Hyprland),
        "sway" => Some(DesktopKind::Sway),
        "i3-wm" => Some(DesktopKind::I3),
        _ => None,
    }
}
fn hint_kind(name: &str) -> Option<DesktopKind> {
    match name.to_ascii_lowercase().as_str() {
        "kde" | "plasma" => Some(DesktopKind::Plasma),
        "gnome" => Some(DesktopKind::Gnome),
        "xfce" => Some(DesktopKind::Xfce),
        "hyprland" => Some(DesktopKind::Hyprland),
        "sway" => Some(DesktopKind::Sway),
        "i3" => Some(DesktopKind::I3),
        _ => None,
    }
}

pub fn detect(
    inventory: &Inventory,
    current_desktop: Option<&str>,
    session_type: Option<&str>,
    root: &Path,
) -> DesktopInventory {
    let mut candidates = BTreeMap::new();
    for package in &inventory.packages {
        if let Some(kind) = package_kind(&package.name) {
            ensure(&mut candidates, kind)
                .installed_packages
                .push(package.name.clone());
        }
    }
    // Only fixed, public session-file paths are checked; file contents are never read.
    let mut session_files: BTreeMap<DesktopKind, (Option<bool>, Option<bool>)> = BTreeMap::new();
    for (kind, dir, file, wayland) in [
        (DesktopKind::Plasma, "xsessions", "plasma.desktop", false),
        (
            DesktopKind::Plasma,
            "wayland-sessions",
            "plasma.desktop",
            true,
        ),
        (DesktopKind::Gnome, "xsessions", "gnome.desktop", false),
        (
            DesktopKind::Gnome,
            "wayland-sessions",
            "gnome.desktop",
            true,
        ),
        (DesktopKind::Xfce, "xsessions", "xfce.desktop", false),
        (
            DesktopKind::Hyprland,
            "wayland-sessions",
            "hyprland.desktop",
            true,
        ),
        (DesktopKind::Sway, "wayland-sessions", "sway.desktop", true),
        (DesktopKind::I3, "xsessions", "i3.desktop", false),
    ] {
        let path = root.join("usr/share").join(dir).join(file);
        let state = match std::fs::metadata(path) {
            Ok(metadata) => Some(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
            Err(_) => None,
        };
        if state == Some(true) {
            ensure(&mut candidates, kind);
        }
        let entry = session_files.entry(kind).or_default();
        if wayland {
            entry.1 = state;
        } else {
            entry.0 = state;
        }
    }
    if let Some(hint) = current_desktop.filter(|s| s.len() <= 1024) {
        for part in hint.split(':') {
            if let Some(kind) = hint_kind(part.trim()) {
                ensure(&mut candidates, kind).current_process_session_hint = true;
            }
        }
    }
    for candidate in candidates.values_mut() {
        if let Some((x11, wayland)) = session_files.get(&candidate.kind) {
            candidate.x11_session_file = *x11;
            candidate.wayland_session_file = *wayland;
        }
        candidate.installed_packages.sort();
        candidate.installed_packages.dedup();
    }
    let current_process_session_type = match session_type.map(str::trim) {
        Some("x11") => Some(SessionType::X11),
        Some("wayland") => Some(SessionType::Wayland),
        _ => None,
    };
    DesktopInventory {
        candidates: candidates.into_values().collect(),
        current_process_session_type,
    }
}

pub fn scan(inventory: &Inventory) -> DesktopInventory {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    detect(
        inventory,
        desktop.as_deref(),
        session.as_deref(),
        Path::new("/"),
    )
}

pub fn enabled_display_managers(inventory: &Inventory) -> Vec<&'static str> {
    let mut out = Vec::new();
    if let Some(scope) = inventory
        .service_scopes
        .iter()
        .find(|scope| scope.scope == Scope::System && scope.unit_files_available)
    {
        for (service, name) in [
            ("sddm.service", "SDDM"),
            ("gdm.service", "GDM"),
            ("lightdm.service", "LightDM"),
            ("lemurs.service", "Lemurs"),
        ] {
            if scope
                .services
                .iter()
                .any(|s| s.name == service && s.unit_file_state.as_deref() == Some("enabled"))
            {
                out.push(name);
            }
        }
    }
    out
}

pub fn suggested_option(kind: DesktopKind) -> Option<&'static str> {
    match kind {
        DesktopKind::Plasma => Some("services.desktopManager.plasma6.enable"),
        DesktopKind::Gnome => Some("services.desktopManager.gnome.enable"),
        DesktopKind::Xfce => Some("services.xserver.desktopManager.xfce.enable"),
        DesktopKind::Hyprland => Some("programs.hyprland.enable"),
        DesktopKind::Sway => Some("programs.sway.enable"),
        DesktopKind::I3 => None,
    }
}
