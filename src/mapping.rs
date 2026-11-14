use crate::inventory::{Inventory, Package};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Mapped,
    Inferred,
    Reviewed,
    Review,
    Unmapped,
    Dependency,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub package: String,
    pub status: Status,
    pub nixpkgs_attribute: Option<String>,
    pub reason: String,
}
/// Versioned, distro-specific data bundled into the binary for offline use.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingDatabase {
    pub schema_version: u32,
    pub distribution: String,
    pub entries: Vec<MappingEntry>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub nixpkgs_revision: String,
    pub method: String,
    pub limitations: String,
    #[serde(default)]
    pub source_homepage: Option<String>,
    #[serde(default)]
    pub nixpkgs_homepage: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingEntry {
    pub source_package: String,
    pub kind: MappingKind,
    pub nixpkgs_attribute: Option<String>,
    pub note: String,
    pub verification: Option<Verification>,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingKind {
    Package,
    ManualReview,
}

pub fn parse_database(json: &str) -> Result<MappingDatabase, String> {
    let db: MappingDatabase =
        serde_json::from_str(json).map_err(|e| format!("invalid mapping database: {e}"))?;
    if db.schema_version != 1 || db.distribution != "manjaro" {
        return Err("unsupported mapping database version or distribution".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for entry in &db.entries {
        if !crate::inventory::valid_package_name(&entry.source_package)
            || !names.insert(&entry.source_package)
        {
            return Err(format!(
                "invalid or duplicate mapping source: {:?}",
                entry.source_package
            ));
        }
        if entry.note.is_empty()
            || entry.note.len() > 1000
            || entry.note.chars().any(char::is_control)
        {
            return Err(format!("invalid mapping note: {:?}", entry.source_package));
        }
        match (&entry.kind, &entry.nixpkgs_attribute, &entry.verification) {
            (MappingKind::Package, Some(attribute), Some(evidence))
                if valid_attribute(attribute) =>
            {
                crate::draft::normalize_revision(&evidence.nixpkgs_revision)?;
                if evidence.method != "local_attribute_is_derivation"
                    || evidence.limitations.is_empty()
                    || evidence.limitations.len() > 1000
                    || evidence.limitations.chars().any(char::is_control)
                    || [&evidence.source_homepage, &evidence.nixpkgs_homepage]
                        .into_iter()
                        .flatten()
                        .any(|url| {
                            url.len() > 2048
                                || url.chars().any(char::is_control)
                                || !(url.starts_with("https://") || url.starts_with("http://"))
                        })
                {
                    return Err(format!(
                        "invalid verification metadata for {:?}",
                        entry.source_package
                    ));
                }
            }
            (MappingKind::ManualReview, None, None) => {}
            _ => {
                return Err(format!(
                    "invalid mapping target for {:?}",
                    entry.source_package
                ))
            }
        }
    }
    Ok(db)
}
fn bundled_database() -> &'static MappingDatabase {
    static DATABASE: std::sync::OnceLock<MappingDatabase> = std::sync::OnceLock::new();
    DATABASE.get_or_init(|| {
        parse_database(include_str!("../mappings/manjaro.json"))
            .expect("bundled Manjaro mappings must be valid")
    })
}
fn curated(name: &str) -> Option<&'static MappingEntry> {
    bundled_database()
        .entries
        .iter()
        .find(|entry| entry.source_package == name)
}

#[derive(Deserialize)]
struct AurCandidates {
    entries: Vec<AurCandidate>,
}

#[derive(Deserialize)]
struct AurCandidate {
    source_package: String,
    nixpkgs_attribute: String,
    evidence: String,
}

fn aur_candidate(name: &str) -> Option<&'static AurCandidate> {
    static CANDIDATES: std::sync::OnceLock<AurCandidates> = std::sync::OnceLock::new();
    CANDIDATES
        .get_or_init(|| {
            serde_json::from_str(include_str!("../mappings/aur-candidates.json"))
                .expect("bundled AUR candidates must be valid JSON")
        })
        .entries
        .iter()
        .find(|entry| entry.source_package == name && valid_attribute(&entry.nixpkgs_attribute))
}
fn manjaro_candidate(name: &str) -> Option<&'static AurCandidate> {
    static CANDIDATES: std::sync::OnceLock<AurCandidates> = std::sync::OnceLock::new();
    CANDIDATES
        .get_or_init(|| {
            serde_json::from_str(include_str!("../mappings/manjaro-candidates.json"))
                .expect("bundled Manjaro candidates must be valid JSON")
        })
        .entries
        .iter()
        .find(|entry| entry.source_package == name && valid_attribute(&entry.nixpkgs_attribute))
}
pub fn decision(package: &Package) -> Decision {
    let (status, attr, reason) = if !package.explicitly_installed {
        (
            Status::Dependency,
            None,
            "Dependency-installed; not added as a direct system package.",
        )
    } else if package.foreign {
        (Status::Review, None, "Foreign package: provenance and equivalence require manual review; foreign does not mean AUR.")
    } else if let Some(entry) = curated(&package.name) {
        match entry.kind {
            MappingKind::Package => (
                Status::Mapped,
                entry.nixpkgs_attribute.clone(),
                entry.note.as_str(),
            ),
            MappingKind::ManualReview => (Status::Review, None, entry.note.as_str()),
        }
    } else {
        (
            Status::Unmapped,
            None,
            "No curated mapping available; research and select a replacement manually.",
        )
    };
    let (status, attr, reason) = if status == Status::Unmapped && package.explicitly_installed {
        match manjaro_candidate(&package.name) {
            Some(candidate) => (Status::Inferred, Some(candidate.nixpkgs_attribute.clone()), "Inferred from Manjaro repository and Nixpkgs metadata: same name and upstream homepage. Verify equivalence, features, licensing and availability at the target revision; catalog checkout revision is self-reported and may differ.".to_owned()),
            None => (status, attr, reason.to_owned()),
        }
    } else if package.explicitly_installed && package.foreign {
        match aur_candidate(&package.name) {
            Some(candidate) if candidate.evidence == "same_name_and_upstream_homepage" => (Status::Inferred, Some(candidate.nixpkgs_attribute.clone()), "Inferred from AUR and Nixpkgs metadata: same name and upstream homepage. Foreign status does not prove AUR provenance; verify provenance, equivalence, licensing and target availability.".to_owned()),
            Some(candidate) => (status, attr, format!("{reason} AUR name-only candidate: Nixpkgs {}; no homepage match. Review before including.", candidate.nixpkgs_attribute)),
            None => (status, attr, reason.to_owned()),
        }
    } else if status == Status::Mapped {
        let evidence = curated(&package.name)
            .and_then(|entry| entry.verification.as_ref())
            .expect("validated package mapping must have verification metadata");
        (status, attr, format!("{} Attribute was observed as a derivation at self-reported Nixpkgs revision {}; this does not prove equivalence or target availability.", reason, evidence.nixpkgs_revision))
    } else {
        (status, attr, reason.to_owned())
    };
    Decision {
        package: package.name.clone(),
        status,
        nixpkgs_attribute: attr,
        reason,
    }
}
pub fn decisions(inventory: &Inventory) -> Vec<Decision> {
    let mut decisions: Vec<_> = inventory.packages.iter().map(decision).collect();
    decisions.sort_by(|a, b| a.package.cmp(&b.package));
    decisions
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedMappings {
    pub schema_version: u32,
    /// Optional user-declared revision; matching it does not authenticate content.
    pub nixpkgs_revision: Option<String>,
    pub mappings: Vec<ReviewedMapping>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedMapping {
    pub source_package: String,
    /// A single top-level Nixpkgs attribute, never an expression or attribute path.
    pub nixpkgs_attribute: String,
}
impl ReviewedMappings {
    pub fn empty() -> Self {
        Self {
            schema_version: 1,
            nixpkgs_revision: None,
            mappings: Vec::new(),
        }
    }
}
pub fn read_reviewed(path: &Path) -> Result<ReviewedMappings, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("cannot open reviewed mappings: {e}"))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read reviewed mappings: {e}"))?;
    if bytes.len() > 1024 * 1024 {
        return Err("reviewed mappings exceed 1 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid reviewed mappings: {e}"))
}
pub(crate) fn valid_attribute(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
}
pub fn reviewed_decisions(
    inventory: &Inventory,
    review: &ReviewedMappings,
    target_revision: Option<&str>,
) -> Result<Vec<Decision>, String> {
    inventory.validate()?;
    if review.schema_version != 1 {
        return Err("unsupported reviewed mappings schema version".into());
    }
    if let Some(revision) = &review.nixpkgs_revision {
        let normalized = crate::draft::normalize_revision(revision)?;
        if target_revision.is_some_and(|target| target != normalized) {
            return Err(
                "reviewed mappings declare a different Nixpkgs revision from the draft".into(),
            );
        }
    }
    let packages: BTreeMap<_, _> = inventory
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let mut overrides = BTreeMap::new();
    for item in &review.mappings {
        if !valid_attribute(&item.nixpkgs_attribute) {
            return Err(format!(
                "invalid top-level Nixpkgs attribute: {:?}",
                item.nixpkgs_attribute
            ));
        }
        let package = packages
            .get(item.source_package.as_str())
            .ok_or_else(|| format!("reviewed package not found: {:?}", item.source_package))?;
        if !package.explicitly_installed {
            return Err(format!(
                "reviewed package was not explicitly installed: {:?}",
                item.source_package
            ));
        }
        if overrides
            .insert(
                item.source_package.as_str(),
                item.nixpkgs_attribute.as_str(),
            )
            .is_some()
        {
            return Err(format!(
                "duplicate reviewed package: {:?}",
                item.source_package
            ));
        }
    }
    let mut result = decisions(inventory);
    for decision in &mut result {
        if let Some(attr) = overrides.get(decision.package.as_str()) {
            decision.status = Status::Reviewed;
            decision.nixpkgs_attribute = Some((*attr).to_string());
            decision.reason = "User-reviewed mapping; Nixpkgs availability and behavioral equivalence are not automatically verified.".into();
        }
    }
    Ok(result)
}
