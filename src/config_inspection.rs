//! Opt-in, bounded inspection of explicitly selected configuration files.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::Path,
};

const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Hyprland,
    DockerDaemon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub schema_version: u32,
    pub profile: Profile,
    pub settings: Vec<Setting>,
    pub unhandled_items: u32,
    pub incomplete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Setting {
    HyprlandLayout(Layout),
    HyprlandNaturalScroll(bool),
    DockerLiveRestore(bool),
}

impl Inspection {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.settings.len() > 2 {
            return Err("unsupported configuration inspection".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for item in &self.settings {
            let (kind, valid) = match item {
                Setting::HyprlandLayout(_) => ("layout", self.profile == Profile::Hyprland),
                Setting::HyprlandNaturalScroll(_) => {
                    ("natural_scroll", self.profile == Profile::Hyprland)
                }
                Setting::DockerLiveRestore(_) => {
                    ("live_restore", self.profile == Profile::DockerDaemon)
                }
            };
            if !valid || !seen.insert(kind) {
                return Err("invalid or duplicate inspected setting".into());
            }
        }
        if self.incomplete && !self.settings.is_empty() {
            return Err("ambiguous inspection cannot contain settings".into());
        }
        Ok(())
    }
}

pub fn inspect(profile: Profile, text: &str) -> Result<Inspection, String> {
    let (settings, unhandled_items, incomplete) = match profile {
        Profile::Hyprland => {
            let parsed = parse_hyprland(text);
            let mut settings = Vec::new();
            if let Some(layout) = parsed.layout {
                settings.push(Setting::HyprlandLayout(layout));
            }
            if let Some(value) = parsed.natural_scroll {
                settings.push(Setting::HyprlandNaturalScroll(value));
            }
            (
                settings,
                parsed.ignored_directives,
                parsed.has_sources || parsed.ambiguous,
            )
        }
        Profile::DockerDaemon => {
            let parsed = parse_docker(text)?;
            let settings = parsed
                .live_restore
                .map(Setting::DockerLiveRestore)
                .into_iter()
                .collect();
            (settings, parsed.ignored_settings, false)
        }
    };
    Ok(Inspection {
        schema_version: 1,
        profile,
        settings,
        unhandled_items,
        incomplete,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hyprland {
    pub layout: Option<Layout>,
    pub natural_scroll: Option<bool>,
    pub ignored_directives: u32,
    pub has_sources: bool,
    pub ambiguous: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Dwindle,
    Master,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Docker {
    pub live_restore: Option<bool>,
    pub ignored_settings: u32,
}

pub fn read_explicit(path: &Path) -> Result<String, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "cannot inspect the selected configuration file")?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_BYTES {
        return Err(
            "selected configuration must be a regular non-symlink file under 256 KiB".into(),
        );
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000);
    } // O_NOFOLLOW
    let mut content = String::new();
    options
        .open(path)
        .map_err(|_| "cannot open selected configuration file")?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|_| "cannot read selected configuration file as UTF-8")?;
    if content.len() as u64 > MAX_BYTES {
        return Err("configuration exceeds 256 KiB".into());
    }
    Ok(content)
}

pub fn parse_hyprland(text: &str) -> Hyprland {
    let mut result = Hyprland {
        layout: None,
        natural_scroll: None,
        ignored_directives: 0,
        has_sources: false,
        ambiguous: false,
    };
    let mut sections = Vec::new();
    let mut seen_layout = false;
    let mut seen_scroll = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.ends_with('{') && !line.contains('=') {
            if sections.len() >= 32 {
                result.ambiguous = true;
                break;
            }
            sections.push(line.trim_end_matches('{').trim());
            continue;
        }
        if line == "}" {
            if sections.pop().is_none() {
                result.ambiguous = true;
            }
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            result.ignored_directives += 1;
            continue;
        };
        let key = key.trim();
        // Only exact allowlisted scalar values are retained. Do not preserve comments.
        let value = raw.split('#').next().unwrap_or("").trim();
        if key == "source" {
            result.has_sources = true;
        }
        match (sections.as_slice(), key) {
            (["general"], "layout") => {
                if seen_layout {
                    result.ambiguous = true;
                    result.layout = None;
                } else {
                    seen_layout = true;
                    result.layout = match value {
                        "dwindle" => Some(Layout::Dwindle),
                        "master" => Some(Layout::Master),
                        _ => None,
                    };
                    if result.layout.is_none() {
                        result.ignored_directives += 1;
                    }
                }
            }
            (["input"], "natural_scroll") => {
                if seen_scroll {
                    result.ambiguous = true;
                    result.natural_scroll = None;
                } else {
                    seen_scroll = true;
                    result.natural_scroll = match value {
                        "true" | "1" => Some(true),
                        "false" | "0" => Some(false),
                        _ => None,
                    };
                    if result.natural_scroll.is_none() {
                        result.ignored_directives += 1;
                    }
                }
            }
            _ => result.ignored_directives += 1,
        }
    }
    if !sections.is_empty() {
        result.ambiguous = true;
    }
    // Included files or complex/duplicate blocks can override these values.
    if result.has_sources || result.ambiguous {
        result.layout = None;
        result.natural_scroll = None;
    }
    result
}

pub fn parse_docker(text: &str) -> Result<Docker, String> {
    use serde::de::{IgnoredAny, MapAccess, Visitor};
    use std::{collections::BTreeSet, fmt};
    struct DockerVisitor;
    impl<'de> Visitor<'de> for DockerVisitor {
        type Value = Docker;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a Docker configuration object")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Docker, M::Error> {
            let mut seen = BTreeSet::new();
            let mut out = Docker {
                live_restore: None,
                ignored_settings: 0,
            };
            while let Some(key) = map.next_key::<String>()? {
                if !seen.insert(key.clone()) {
                    return Err(serde::de::Error::custom("duplicate Docker key"));
                }
                if key == "live-restore" {
                    out.live_restore = Some(map.next_value::<bool>()?);
                } else {
                    map.next_value::<IgnoredAny>()?;
                    out.ignored_settings = out.ignored_settings.saturating_add(1);
                }
            }
            Ok(out)
        }
    }
    let mut de = serde_json::Deserializer::from_str(text);
    let result = serde::de::Deserializer::deserialize_map(&mut de, DockerVisitor)
        .map_err(|_| "invalid or ambiguous Docker configuration")?;
    de.end()
        .map_err(|_| "invalid trailing Docker configuration")?;
    Ok(result)
}
