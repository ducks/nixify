//! Offline suggestions from Nixpkgs packages.json metadata.
use crate::{
    inventory::Inventory,
    mapping::{self, Status},
};
use serde::{
    de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor},
    Serialize,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

const MAX_CATALOG: u64 = 1024 * 1024 * 1024;
const MAX_PER_PACKAGE: usize = 10;
type IndexEntry = (String, Option<String>, Option<String>, Option<String>);

#[derive(Debug, Serialize)]
pub struct CatalogReport {
    pub schema_version: u32,
    pub catalog_revision_declared: String,
    pub target_system: String,
    pub matches: Vec<PackageMatches>,
    pub warning: &'static str,
}
#[derive(Debug, Serialize)]
pub struct PackageMatches {
    pub source_package: String,
    pub foreign: bool,
    pub candidates: Vec<Candidate>,
    pub additional_candidates: usize,
}
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub attribute: String,
    pub match_kind: &'static str,
    pub pname: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
}
fn text(value: Option<&Value>, max: usize) -> Option<String> {
    let value = value?.as_str()?;
    if value.chars().any(char::is_control) {
        return None;
    }
    let value: String = value.chars().take(max).collect();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

// These are search hints only; names from different distributions can collide.
fn probes(name: &str, foreign: bool) -> Vec<(String, &'static str)> {
    let name = name.to_ascii_lowercase();
    let mut results = vec![(name.clone(), "exact")];
    if foreign {
        for suffix in ["-bin", "-git"] {
            if let Some(base) = name.strip_suffix(suffix).filter(|s| !s.is_empty()) {
                results.push((base.to_string(), "foreign_suffix_removed"));
            }
        }
    }
    for prefix in ["python-", "xorg-", "ttf-"] {
        if let Some(base) = name.strip_prefix(prefix).filter(|s| !s.is_empty()) {
            results.push((base.to_string(), "distribution_prefix_removed"));
        }
    }
    if ["grep", "sed", "tar"].contains(&name.as_str()) {
        results.push((format!("gnu{name}"), "gnu_prefix_added"));
    }
    if name == "procps-ng" {
        results.push(("procps".into(), "known_name_variant"));
    }
    results
}

pub fn from_value(
    inventory: &Inventory,
    catalog: &Value,
    revision: &str,
) -> Result<CatalogReport, String> {
    inventory.validate()?;
    let revision = crate::draft::normalize_revision(revision)?;
    let system = match inventory.source.architecture.as_str() {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        _ => return Err("unsupported source architecture".into()),
    };
    let root = catalog
        .as_object()
        .ok_or("packages.json must be a JSON object")?;
    // Official channel archives wrap the nix-env attribute map in `packages`.
    // Bare nix-env --json output is also accepted for local snapshots.
    let packages = match root.get("packages") {
        Some(Value::Object(map))
            if !map.contains_key("meta")
                && !map.contains_key("pname")
                && !map.contains_key("name") =>
        {
            map
        }
        Some(Value::Object(_)) | None => root,
        Some(_) => return Err("catalog packages field must be an object".into()),
    };
    // Index exact names only; inputs remain hints and are never rendered as Nix.
    let mut index: BTreeMap<String, Vec<IndexEntry>> = BTreeMap::new();
    for (attribute, package) in packages {
        if !mapping::valid_attribute(attribute) {
            continue;
        }
        let Some(record) = package.as_object() else {
            continue;
        };
        let pname =
            text(record.get("pname"), 255).filter(|p| crate::inventory::valid_package_name(p));
        let version = text(record.get("version"), 128);
        let description = text(record.get("meta").and_then(|m| m.get("description")), 300);
        let mut keys = BTreeSet::new();
        if let Some(leaf) = attribute.rsplit('.').next() {
            keys.insert(leaf.to_ascii_lowercase());
        }
        if let Some(name) = &pname {
            keys.insert(name.to_ascii_lowercase());
        }
        for key in keys {
            index.entry(key).or_default().push((
                attribute.clone(),
                pname.clone(),
                version.clone(),
                description.clone(),
            ));
        }
    }
    let unresolved: BTreeSet<_> = mapping::decisions(inventory)
        .into_iter()
        .filter(|d| matches!(d.status, Status::Unmapped | Status::Review))
        .map(|d| d.package)
        .collect();
    let mut matches = Vec::new();
    for package in inventory
        .packages
        .iter()
        .filter(|p| p.explicitly_installed && unresolved.contains(&p.name))
    {
        let raw = package.name.to_ascii_lowercase();
        let mut found = BTreeMap::new();
        for (key, probe_kind) in probes(&raw, package.foreign) {
            if let Some(items) = index.get(&key) {
                for (attribute, pname, version, description) in items {
                    let leaf = attribute.rsplit('.').next().unwrap_or("");
                    let kind = if probe_kind != "exact" {
                        probe_kind
                    } else if leaf.eq_ignore_ascii_case(&raw) {
                        "attribute_leaf"
                    } else {
                        "package_name"
                    };
                    found.entry(attribute.clone()).or_insert_with(|| Candidate {
                        attribute: attribute.clone(),
                        match_kind: kind,
                        pname: pname.clone(),
                        version: version.clone(),
                        description: description.clone(),
                    });
                }
            }
        }
        let mut candidates: Vec<_> = found.into_values().collect();
        candidates.sort_by(|a, b| {
            let rank = |k| match k {
                "attribute_leaf" => 0,
                "package_name" => 1,
                _ => 2,
            };
            rank(a.match_kind)
                .cmp(&rank(b.match_kind))
                .then(a.attribute.cmp(&b.attribute))
        });
        let additional_candidates = candidates.len().saturating_sub(MAX_PER_PACKAGE);
        candidates.truncate(MAX_PER_PACKAGE);
        matches.push(PackageMatches {
            source_package: package.name.clone(),
            foreign: package.foreign,
            candidates,
            additional_candidates,
        });
    }
    matches.sort_by(|a, b| a.source_package.cmp(&b.source_package));
    Ok(CatalogReport { schema_version: 1, catalog_revision_declared: revision, target_system: system.into(), matches, warning: "Catalog names are unreviewed candidates, not equivalence, installability, provenance, or evidence the attribute exists on this target. Catalog revision is user-declared and not authenticated. No mappings were accepted." })
}

struct PackageSeed<'a> {
    wanted: &'a BTreeSet<String>,
}
impl<'de> DeserializeSeed<'de> for PackageSeed<'_> {
    type Value = serde_json::Map<String, Value>;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        struct PackageVisitor<'a> {
            wanted: &'a BTreeSet<String>,
        }
        impl<'de> Visitor<'de> for PackageVisitor<'_> {
            type Value = serde_json::Map<String, Value>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a package metadata object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut retained = serde_json::Map::new();
                while let Some(attribute) = map.next_key::<String>()? {
                    if !mapping::valid_attribute(&attribute) {
                        map.next_value::<IgnoredAny>()?;
                        continue;
                    }
                    let record: Value = map.next_value()?;
                    let leaf = attribute
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let pname = record
                        .get("pname")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if self.wanted.contains(&leaf) || self.wanted.contains(&pname) {
                        retained.insert(attribute, record);
                    }
                }
                Ok(retained)
            }
        }
        deserializer.deserialize_map(PackageVisitor {
            wanted: self.wanted,
        })
    }
}
struct ChannelSeed<'a> {
    wanted: &'a BTreeSet<String>,
}
impl<'de> DeserializeSeed<'de> for ChannelSeed<'_> {
    type Value = serde_json::Map<String, Value>;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        struct ChannelVisitor<'a> {
            wanted: &'a BTreeSet<String>,
        }
        impl<'de> Visitor<'de> for ChannelVisitor<'_> {
            type Value = serde_json::Map<String, Value>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a channel catalog with a packages field")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut packages = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "packages" {
                        if packages.is_some() {
                            return Err(serde::de::Error::custom("duplicate packages field"));
                        }
                        packages = Some(map.next_value_seed(PackageSeed {
                            wanted: self.wanted,
                        })?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                packages.ok_or_else(|| serde::de::Error::custom("missing packages field"))
            }
        }
        deserializer.deserialize_map(ChannelVisitor {
            wanted: self.wanted,
        })
    }
}

pub fn read(inventory: &Inventory, path: &Path, revision: &str) -> Result<CatalogReport, String> {
    inventory.validate()?;
    crate::draft::normalize_revision(revision)?;
    let unresolved: BTreeSet<_> = mapping::decisions(inventory)
        .into_iter()
        .filter(|d| matches!(d.status, Status::Unmapped | Status::Review))
        .map(|d| d.package)
        .collect();
    let mut wanted = BTreeSet::new();
    for package in inventory
        .packages
        .iter()
        .filter(|p| p.explicitly_installed && unresolved.contains(&p.name))
    {
        wanted.extend(
            probes(&package.name, package.foreign)
                .into_iter()
                .map(|(name, _)| name),
        );
    }
    let file = File::open(path).map_err(|e| format!("cannot open catalog: {e}"))?;
    if file
        .metadata()
        .map_err(|e| format!("cannot inspect catalog: {e}"))?
        .len()
        > MAX_CATALOG
    {
        return Err("catalog exceeds 1 GiB".into());
    }
    let reader = BufReader::new(file).take(MAX_CATALOG + 1);
    let mut de = serde_json::Deserializer::from_reader(reader);
    let filtered = ChannelSeed { wanted: &wanted }
        .deserialize(&mut de)
        .map_err(|_| "invalid channel packages.json; expected an object with a packages field")?;
    de.end().map_err(|_| "invalid trailing catalog data")?;
    from_value(inventory, &Value::Object(filtered), revision)
}
