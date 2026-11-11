use crate::{
    inventory::{valid_service_name, Scope, Service, ServiceScope},
    manjaro::run_bounded,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn parse_unit_files(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or("");
        let state = fields.next().unwrap_or("");
        if !valid_service_name(name)
            || state.is_empty()
            || state.len() > 32
            || !state.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
            || result.insert(name.into(), state.into()).is_some()
        {
            return Err("malformed systemctl list-unit-files output".into());
        }
    }
    Ok(result)
}
pub fn parse_running(text: &str) -> Result<BTreeSet<String>, String> {
    let mut result = BTreeSet::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or("");
        let load = fields.next().unwrap_or("");
        let active = fields.next().unwrap_or("");
        let sub = fields.next().unwrap_or("");
        if !valid_service_name(name)
            || load.is_empty()
            || active != "active"
            || sub != "running"
            || !result.insert(name.into())
        {
            return Err("malformed systemctl running-units output".into());
        }
    }
    Ok(result)
}
fn query(scope: Scope, kind: &str) -> Result<String, crate::manjaro::ScanError> {
    let mut args = Vec::new();
    if scope == Scope::User {
        args.push("--user");
    }
    args.push(kind);
    args.extend([
        "--type=service",
        "--no-pager",
        "--no-legend",
        "--plain",
        "--no-ask-password",
    ]);
    if kind == "list-units" {
        args.push("--state=running");
    }
    run_bounded("systemctl", &args, false)
}
pub fn from_outputs(scope: Scope, files: Option<&str>, running: Option<&str>) -> ServiceScope {
    // Any malformed output invalidates the entire respective query; partial
    // results must never be represented as a complete inventory.
    let files = files.and_then(|s| parse_unit_files(s).ok());
    let running = running.and_then(|s| parse_running(s).ok());
    let mut names = BTreeSet::new();
    if let Some(files) = &files {
        names.extend(files.keys().cloned());
    }
    if let Some(running) = &running {
        names.extend(running.iter().cloned());
    }
    let services = names
        .into_iter()
        .map(|name| {
            let unit_file_state = files.as_ref().and_then(|f| f.get(&name).cloned());
            let running_state = running.as_ref().map(|r| r.contains(&name));
            Service {
                name,
                unit_file_state,
                running: running_state,
                evidence: vec![
                    format!(
                        "systemctl {}list-unit-files",
                        if scope == Scope::User { "--user " } else { "" }
                    ),
                    format!(
                        "systemctl {}list-units --state=running",
                        if scope == Scope::User { "--user " } else { "" }
                    ),
                ],
            }
        })
        .collect();
    ServiceScope {
        scope,
        unit_files_available: files.is_some(),
        runtime_available: running.is_some(),
        services,
    }
}
pub fn scan() -> Vec<ServiceScope> {
    [Scope::System, Scope::User]
        .into_iter()
        .map(|scope| {
            let files = query(scope, "list-unit-files").ok();
            let running = query(scope, "list-units").ok();
            from_outputs(scope, files.as_deref(), running.as_deref())
        })
        .collect()
}
