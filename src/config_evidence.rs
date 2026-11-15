//! Fixed-location configuration discovery. Never reads configuration contents.
use crate::inventory::Inventory;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigId {
    Sshd,
    Caddy,
    Nginx,
    Docker,
    Podman,
    Lemurs,
    Hyprland,
    Waybar,
    WaybarJsonc,
    Kitty,
    Neovim,
    NeovimVim,
    Zsh,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigState {
    Present,
    Missing,
    SymlinkNotFollowed,
    OtherType,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigEvidence {
    pub id: ConfigId,
    pub state: ConfigState,
}

impl ConfigId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sshd => "/etc/ssh/sshd_config",
            Self::Caddy => "/etc/caddy/Caddyfile",
            Self::Nginx => "/etc/nginx/nginx.conf",
            Self::Docker => "/etc/docker/daemon.json",
            Self::Podman => "/etc/containers/containers.conf",
            Self::Lemurs => "/etc/lemurs/config.toml",
            Self::Hyprland => "$HOME/.config/hypr/hyprland.conf",
            Self::Waybar => "$HOME/.config/waybar/config",
            Self::WaybarJsonc => "$HOME/.config/waybar/config.jsonc",
            Self::Kitty => "$HOME/.config/kitty/kitty.conf",
            Self::Neovim => "$HOME/.config/nvim/init.lua",
            Self::NeovimVim => "$HOME/.config/nvim/init.vim",
            Self::Zsh => "$HOME/.zshrc",
        }
    }
}

// Reject symlinks in every component. Inspect metadata only; never read files.
fn inspect(root: &Path, relative: &str) -> ConfigState {
    if !root.is_absolute() {
        return ConfigState::Unavailable;
    }
    let mut path = root.to_path_buf();
    for (i, part) in relative.split('/').enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => return ConfigState::SymlinkNotFollowed,
            Ok(m) if i + 1 == relative.split('/').count() => {
                return if m.is_file() {
                    ConfigState::Present
                } else {
                    ConfigState::OtherType
                }
            }
            Ok(m) if !m.is_dir() => return ConfigState::OtherType,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ConfigState::Missing,
            Err(_) => return ConfigState::Unavailable,
        }
    }
    ConfigState::Unavailable
}

pub fn scan_at(
    inventory: &Inventory,
    system_root: &Path,
    home: Option<&Path>,
) -> Vec<ConfigEvidence> {
    let specs: &[(ConfigId, &str, bool, &[&str])] = &[
        (ConfigId::Sshd, "etc/ssh/sshd_config", false, &["openssh"]),
        (ConfigId::Caddy, "etc/caddy/Caddyfile", false, &["caddy"]),
        (ConfigId::Nginx, "etc/nginx/nginx.conf", false, &["nginx"]),
        (
            ConfigId::Docker,
            "etc/docker/daemon.json",
            false,
            &["docker"],
        ),
        (
            ConfigId::Podman,
            "etc/containers/containers.conf",
            false,
            &["podman"],
        ),
        (
            ConfigId::Lemurs,
            "etc/lemurs/config.toml",
            false,
            &["lemurs"],
        ),
        (
            ConfigId::Hyprland,
            ".config/hypr/hyprland.conf",
            true,
            &["hyprland"],
        ),
        (ConfigId::Waybar, ".config/waybar/config", true, &["waybar"]),
        (
            ConfigId::WaybarJsonc,
            ".config/waybar/config.jsonc",
            true,
            &["waybar"],
        ),
        (
            ConfigId::Kitty,
            ".config/kitty/kitty.conf",
            true,
            &["kitty"],
        ),
        (ConfigId::Neovim, ".config/nvim/init.lua", true, &["neovim"]),
        (
            ConfigId::NeovimVim,
            ".config/nvim/init.vim",
            true,
            &["neovim"],
        ),
        (ConfigId::Zsh, ".zshrc", true, &["zsh"]),
    ];
    let mut result = Vec::new();
    for &(id, path, user, names) in specs {
        if !inventory
            .packages
            .iter()
            .any(|p| names.contains(&p.name.as_str()))
        {
            continue;
        }
        let root = if user { home } else { Some(system_root) };
        result.push(ConfigEvidence {
            id,
            state: root.map_or(ConfigState::Unavailable, |root| inspect(root, path)),
        });
    }
    result.sort_by_key(|item| item.id);
    result
}

pub fn scan(inventory: &Inventory) -> Vec<ConfigEvidence> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    scan_at(inventory, Path::new("/"), home.as_deref())
}
