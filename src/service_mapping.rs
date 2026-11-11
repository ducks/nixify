use crate::inventory::{Inventory, Scope};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ServiceDecision {
    pub scope: Scope,
    pub service: String,
    pub unit_file_state: Option<String>,
    pub running: Option<bool>,
    pub nixos_option: Option<&'static str>,
    pub reason: &'static str,
}
fn mapped(name: &str) -> Option<&'static str> {
    match name {
        "sshd.service" => Some("services.openssh.enable"),
        "NetworkManager.service" => Some("networking.networkmanager.enable"),
        "nginx.service" => Some("services.nginx.enable"),
        "caddy.service" => Some("services.caddy.enable"),
        "docker.service" => Some("virtualisation.docker.enable"),
        "postgresql.service" => Some("services.postgresql.enable"),
        _ => None,
    }
}
pub fn decisions(inventory: &Inventory) -> Vec<ServiceDecision> {
    let mut decisions = Vec::new();
    for scope in &inventory.service_scopes {
        for service in &scope.services {
            // Only persistent enablement is translated; currently running,
            // enabled-runtime, static, user and unknown states require review.
            if service.unit_file_state.as_deref() != Some("enabled")
                && service.running != Some(true)
                && service.unit_file_state.as_deref() != Some("enabled-runtime")
            {
                continue;
            }
            let attr = if scope.scope == Scope::System
                && service.unit_file_state.as_deref() == Some("enabled")
            {
                mapped(&service.name)
            } else {
                None
            };
            let reason = if attr.is_some() {
                "Proposed module enablement only; configuration, data, security settings and target availability require review."
            } else if scope.scope == Scope::User {
                "User service requires manual migration; no system service was generated."
            } else if service.unit_file_state.as_deref() != Some("enabled") {
                "Running or runtime-enabled is not persistent enablement; review manually."
            } else {
                "No curated service mapping; review the unit and its configuration manually."
            };
            decisions.push(ServiceDecision {
                scope: scope.scope,
                service: service.name.clone(),
                unit_file_state: service.unit_file_state.clone(),
                running: service.running,
                nixos_option: attr,
                reason,
            });
        }
    }
    decisions.sort_by(|a, b| a.scope.cmp(&b.scope).then(a.service.cmp(&b.service)));
    decisions
}
