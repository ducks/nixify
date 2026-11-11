use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema_version: u32,
    pub source: Source,
    pub packages: Vec<Package>,
    #[serde(default)]
    pub service_scopes: Vec<ServiceScope>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub distribution: String,
    pub architecture: String,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub explicitly_installed: bool,
    pub foreign: bool,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceScope {
    pub scope: Scope,
    pub unit_files_available: bool,
    pub runtime_available: bool,
    pub services: Vec<Service>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    System,
    User,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub name: String,
    pub unit_file_state: Option<String>,
    pub running: Option<bool>,
    pub evidence: Vec<String>,
}
pub fn valid_service_name(name: &str) -> bool {
    name.ends_with(".service")
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+-:".contains(&b) || b == 92)
}
impl Inventory {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported inventory schema version {}",
                self.schema_version
            ));
        }
        if self.source.distribution != "manjaro" {
            return Err(format!(
                "unsupported distribution: {}",
                self.source.distribution
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for package in &self.packages {
            if !valid_package_name(&package.name)
                || package.version.is_empty()
                || package.version.chars().any(char::is_whitespace)
            {
                return Err(format!("invalid package record: {:?}", package.name));
            }
            if !seen.insert(&package.name) {
                return Err(format!("duplicate package: {}", package.name));
            }
        }
        let mut scopes = std::collections::BTreeSet::new();
        for scope in &self.service_scopes {
            if !scopes.insert(scope.scope) {
                return Err(format!("duplicate service scope: {:?}", scope.scope));
            }
            let mut names = std::collections::BTreeSet::new();
            for service in &scope.services {
                if !valid_service_name(&service.name) || !names.insert(&service.name) {
                    return Err(format!(
                        "invalid or duplicate service name: {:?}",
                        service.name
                    ));
                }
                if !scope.runtime_available && service.running.is_some() {
                    return Err(format!("runtime state unavailable for {}", service.name));
                }
                if !scope.unit_files_available && service.unit_file_state.is_some() {
                    return Err(format!("unit file state unavailable for {}", service.name));
                }
                if service.unit_file_state.as_ref().is_some_and(|state| {
                    state.is_empty()
                        || state.len() > 32
                        || !state.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
                }) {
                    return Err(format!("invalid service state for {}", service.name));
                }
            }
        }
        Ok(())
    }
}
pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}
