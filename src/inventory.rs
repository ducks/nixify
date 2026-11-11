use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema_version: u32,
    pub source: Source,
    pub packages: Vec<Package>,
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
