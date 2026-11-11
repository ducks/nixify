use nixify::{
    inventory::Inventory,
    manjaro,
    mapping::{self, Status},
    output,
};
use std::{fs, process::Command};
const ALL: &str = include_str!("fixtures/pacman-q.txt");
const EXPLICIT: &str = include_str!("fixtures/pacman-explicit.txt");
const FOREIGN: &str = include_str!("fixtures/pacman-foreign.txt");
fn fixture() -> Inventory {
    manjaro::inventory(
        "NAME=Manjaro\nID=manjaro\nID_LIKE=arch\nPASSWORD=not-in-output\n",
        "x86_64",
        ALL,
        EXPLICIT,
        FOREIGN,
    )
    .unwrap()
}
#[test]
fn scan_fixture_records_provenance_without_secrets() {
    let inv = fixture();
    assert_eq!(inv.packages.len(), 5);
    assert!(
        !inv.packages
            .iter()
            .find(|p| p.name == "libexample")
            .unwrap()
            .explicitly_installed
    );
    assert!(
        inv.packages
            .iter()
            .find(|p| p.name == "foreign-tool")
            .unwrap()
            .foreign
    );
    let json = serde_json::to_string(&inv).unwrap();
    assert!(!json.contains("not-in-output"));
    assert!(json.contains("pacman -Qqm"));
}
#[test]
fn distro_must_be_manjaro_not_merely_arch_like() {
    assert!(manjaro::inventory(
        "ID=arch\nID_LIKE=manjaro\n",
        "x86_64",
        ALL,
        EXPLICIT,
        FOREIGN
    )
    .is_err());
    assert_eq!(
        manjaro::distro_id("ID=\"manjaro\"\n").as_deref(),
        Some("manjaro")
    );
}
#[test]
fn rejects_malformed_or_inconsistent_queries() {
    assert!(manjaro::inventory("ID=manjaro", "x86_64", "foo 1 extra", "foo", "").is_err());
    assert!(manjaro::inventory("ID=manjaro", "x86_64", "foo 1", "missing", "").is_err());
    assert!(manjaro::inventory("ID=manjaro", "x86_64", "foo 1\nfoo 2", "", "").is_err());
    assert!(manjaro::inventory("ID=manjaro", "x86_64", "foo 1", "foo;bad", "").is_err());
}
#[test]
fn conservative_mapping_and_deterministic_output() {
    let mut inv = fixture();
    let d = mapping::decisions(&inv);
    assert_eq!(
        d.iter().find(|x| x.package == "git").unwrap().status,
        Status::Mapped
    );
    assert_eq!(
        d.iter()
            .find(|x| x.package == "foreign-tool")
            .unwrap()
            .status,
        Status::Review
    );
    assert_eq!(
        d.iter().find(|x| x.package == "libexample").unwrap().status,
        Status::Dependency
    );
    let a = output::packages_nix(&d);
    inv.packages.reverse();
    assert_eq!(a, output::packages_nix(&mapping::decisions(&inv)));
    assert!(!a.contains("foreign-tool"));
    assert!(!a.contains("libexample"));
}
#[test]
fn schema_version_and_injection_are_rejected() {
    let mut inv = fixture();
    inv.schema_version = 2;
    assert!(inv.validate().is_err());
    inv.schema_version = 1;
    inv.packages[0].name = "x; builtins.abort".into();
    assert!(inv.validate().is_err());
}
#[test]
fn cli_generates_and_reports_without_nix() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("inventory.json");
    fs::write(&file, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    let target = tmp.path().join("draft");
    let binary = env!("CARGO_BIN_EXE_nixify");
    let status = Command::new(binary)
        .args([
            "generate",
            "--input",
            file.to_str().unwrap(),
            "--output",
            target.to_str().unwrap(),
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let text = fs::read_to_string(target.join("packages.nix")).unwrap();
    assert!(text.contains("    git\n"));
    assert!(!text.contains("foreign-tool"));
    let report = Command::new(binary)
        .args(["report", "--input", file.to_str().unwrap()])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(report.status.success());
    assert!(String::from_utf8_lossy(&report.stdout).contains("foreign-tool"));
    let again = Command::new(binary)
        .args([
            "generate",
            "--input",
            file.to_str().unwrap(),
            "--output",
            target.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!again.status.success());
    assert_eq!(
        fs::read_to_string(target.join("packages.nix")).unwrap(),
        text
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(target.join("inventory.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn baseline_overlap_does_not_claim_equivalence_or_change_generation() {
    use nixify::baseline::{self, Baseline};
    let inventory = fixture();
    let baseline = Baseline {
        schema_version: 1,
        nixpkgs_path: "/nix/store/example-nixpkgs".into(),
        nixpkgs_version: "24.11".into(),
        system: "x86_64-linux".into(),
        packages: vec!["git".into(), "libexample".into()],
    };
    assert_eq!(baseline::matches(&inventory, &baseline), vec!["git"]);
    let before = output::packages_nix(&mapping::decisions(&inventory));
    assert!(before.contains("    git\n"));
    let temp = tempfile::tempdir().unwrap();
    let inv_path = temp.path().join("inventory.json");
    let baseline_path = temp.path().join("baseline.json");
    fs::write(&inv_path, serde_json::to_vec(&inventory).unwrap()).unwrap();
    fs::write(&baseline_path, serde_json::to_vec(&baseline).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .args([
            "report",
            "--input",
            inv_path.to_str().unwrap(),
            "--baseline",
            baseline_path.to_str().unwrap(),
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("1 explicit source package names match"));
    assert!(text.contains("do not prove equivalence"));
    assert!(text.contains("`git`"));
}

#[test]
fn systemd_distinguishes_persistent_enablement_and_running_state() {
    use nixify::{inventory::Scope, service_mapping, systemd};
    let mut inv = fixture();
    inv.service_scopes = vec![
        systemd::from_outputs(Scope::System, Some("sshd.service enabled enabled\nnginx.service disabled disabled\nNetworkManager.service enabled enabled\ncustom.service enabled enabled\n"), Some("nginx.service loaded active running Web server\nsshd.service loaded active running OpenSSH\n")),
        systemd::from_outputs(Scope::User, Some("caddy.service enabled enabled\n"), None),
    ];
    inv.validate().unwrap();
    let decisions = service_mapping::decisions(&inv);
    let ssh = decisions
        .iter()
        .find(|d| d.service == "sshd.service")
        .unwrap();
    assert_eq!(ssh.nixos_option, Some("services.openssh.enable"));
    assert_eq!(ssh.running, Some(true));
    let nginx = decisions
        .iter()
        .find(|d| d.service == "nginx.service")
        .unwrap();
    assert_eq!(nginx.nixos_option, None);
    assert_eq!(nginx.running, Some(true));
    let user = decisions
        .iter()
        .find(|d| d.service == "caddy.service")
        .unwrap();
    assert_eq!(user.nixos_option, None);
    assert_eq!(user.running, None);
    let script = output::services_nix(&decisions);
    assert!(script.contains("services.openssh.enable = true;"));
    assert!(script.contains("networking.networkmanager.enable = true;"));
    assert!(!script.contains("services.nginx.enable"));
    assert!(!script.contains("services.caddy.enable"));
    let report = output::report(&inv, &mapping::decisions(&inv));
    assert!(report.contains("custom.service"));
    assert!(report.contains("runtime;") || report.contains("running-state unavailable"));
}

#[test]
fn malformed_or_unavailable_systemd_queries_remain_unknown() {
    use nixify::{inventory::Scope, systemd};
    let scope = systemd::from_outputs(
        Scope::System,
        Some("invalid extra data"),
        Some("sshd.service loaded active running SSH"),
    );
    assert!(!scope.unit_files_available);
    assert!(scope.runtime_available);
    assert_eq!(scope.services[0].unit_file_state, None);
    assert_eq!(scope.services[0].running, Some(true));
    let unknown = systemd::from_outputs(Scope::User, None, None);
    assert!(!unknown.runtime_available && !unknown.unit_files_available);
}

#[test]
fn inventory_from_previous_milestone_is_still_readable() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value.as_object_mut().unwrap().remove("service_scopes");
    value.as_object_mut().unwrap().remove("desktop");
    let decoded: Inventory = serde_json::from_value(value).unwrap();
    decoded.validate().unwrap();
    assert!(decoded.service_scopes.is_empty());
}

#[test]
fn desktop_signals_remain_distinct_and_suggestions_are_inert() {
    use nixify::{
        desktop::{self, DesktopKind, SessionType},
        inventory::{Package, Scope},
        systemd,
    };
    let mut inv = fixture();
    inv.packages.push(Package {
        name: "gnome-shell".into(),
        version: "1".into(),
        explicitly_installed: true,
        foreign: false,
        evidence: vec![],
    });
    inv.packages.push(Package {
        name: "hyprland".into(),
        version: "1".into(),
        explicitly_installed: true,
        foreign: false,
        evidence: vec![],
    });
    inv.service_scopes = vec![systemd::from_outputs(
        Scope::System,
        Some("gdm.service enabled enabled\n"),
        None,
    )];
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("usr/share/wayland-sessions")).unwrap();
    fs::write(
        root.path()
            .join("usr/share/wayland-sessions/hyprland.desktop"),
        "PRIVATE=not-read",
    )
    .unwrap();
    let desktop = desktop::detect(&inv, Some("Hyprland"), Some("wayland"), root.path());
    assert_eq!(
        desktop.current_process_session_type,
        Some(SessionType::Wayland)
    );
    let hypr = desktop
        .candidates
        .iter()
        .find(|x| x.kind == DesktopKind::Hyprland)
        .unwrap();
    assert!(hypr.current_process_session_hint && hypr.wayland_session_file == Some(true));
    let gnome = desktop
        .candidates
        .iter()
        .find(|x| x.kind == DesktopKind::Gnome)
        .unwrap();
    assert!(!gnome.current_process_session_hint);
    inv.desktop = Some(desktop);
    assert_eq!(desktop::enabled_display_managers(&inv), vec!["GDM"]);
    let module = output::desktop_nix(&inv);
    assert!(module.contains("# programs.hyprland.enable = true;"));
    assert!(module.contains("# services.desktopManager.gnome.enable = true;"));
    assert!(!module
        .lines()
        .any(|line| line.trim_start().starts_with("programs.hyprland")));
    assert!(!serde_json::to_string(&inv)
        .unwrap()
        .contains("PRIVATE=not-read"));
}

#[test]
fn unrecognized_session_input_is_not_retained_or_guessed() {
    let inv = fixture();
    let root = tempfile::tempdir().unwrap();
    let desktop = nixify::desktop::detect(
        &inv,
        Some("secret-token"),
        Some("secret-token"),
        root.path(),
    );
    assert!(desktop.candidates.is_empty());
    assert!(desktop.current_process_session_type.is_none());
    assert!(!serde_json::to_string(&desktop)
        .unwrap()
        .contains("secret-token"));
}

#[test]
fn draft_pins_revision_and_does_not_invent_hardware_or_lock() {
    let inv = fixture();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("draft");
    let rev = "0123456789abcdef0123456789abcdef01234567";
    output::generate_with_revision(&inv, &dir, rev).unwrap();
    let flake = fs::read_to_string(dir.join("flake.nix")).unwrap();
    assert!(flake.contains(rev));
    assert!(flake.contains("x86_64-linux"));
    assert!(!dir.join("flake.lock").exists());
    assert!(!dir.join("hardware-configuration.nix").exists());
    let config = fs::read_to_string(dir.join("configuration.nix")).unwrap();
    assert!(config.contains("not a bootable or installable"));
    assert!(!config
        .lines()
        .any(|line| line.trim_start().starts_with("system.stateVersion =")));
    let invalid_dir = temp.path().join("invalid");
    assert!(output::generate_with_revision(&inv, &invalid_dir, "bad\"; dangerous").is_err());
    assert!(!invalid_dir.exists());
}

#[test]
fn syntax_validation_checks_all_files_and_stops_on_failure() {
    use nixify::draft;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("draft");
    output::generate(&fixture(), &dir).unwrap();
    let mut checked = Vec::new();
    draft::validate_syntax_with(&dir, |path| {
        checked.push(path.file_name().unwrap().to_owned());
        Ok(())
    })
    .unwrap();
    assert_eq!(checked.len(), draft::NIX_FILES.len());
    let err = draft::validate_syntax_with(&dir, |path| {
        if path.ends_with("packages.nix") {
            Err("bad syntax".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert_eq!(err, "bad syntax");
    fs::remove_file(dir.join("desktop.nix")).unwrap();
    assert!(draft::validate_syntax_with(&dir, |_| Ok(())).is_err());
}

#[cfg(unix)]
#[test]
fn syntax_validation_refuses_symlinked_input_files() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("draft");
    output::generate(&fixture(), &dir).unwrap();
    fs::remove_file(dir.join("desktop.nix")).unwrap();
    symlink(dir.join("packages.nix"), dir.join("desktop.nix")).unwrap();
    assert!(nixify::draft::validate_syntax_with(&dir, |_| Ok(()))
        .unwrap_err()
        .contains("non-symlink"));
}

#[test]
fn attribute_check_requires_unedited_generated_inputs() {
    let inventory = fixture();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("draft");
    output::generate(&inventory, &dir).unwrap();
    let (revision, attrs) = nixify::draft::expected_attributes(&dir, &inventory).unwrap();
    assert_eq!(revision.len(), 40);
    assert_eq!(attrs, vec!["git", "neovim"]);
    let packages = dir.join("packages.nix");
    let original = fs::read_to_string(&packages).unwrap();
    fs::write(&packages, format!("{original}# manual edit\n")).unwrap();
    assert!(nixify::draft::expected_attributes(&dir, &inventory)
        .unwrap_err()
        .contains("edited"));
    fs::write(&packages, original).unwrap();
    let flake = dir.join("flake.nix");
    let original = fs::read_to_string(&flake).unwrap();
    fs::write(&flake, format!("{original}# manual edit\n")).unwrap();
    assert!(nixify::draft::expected_attributes(&dir, &inventory)
        .unwrap_err()
        .contains("edited"));
}

#[test]
fn module_evaluation_is_explicit_and_preserves_failed_assertions() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let nixpkgs = temp.path().join("nixpkgs");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&nixpkgs).unwrap();
    let result = nixify::draft::evaluate_modules_with(&project, &nixpkgs, &fixture(), |args| {
        assert!(args.contains(&"--eval"));
        assert!(!args.contains(&"--build"));
        assert!(!args.iter().any(|s| s.contains("switch-to-configuration")));
        Ok(r#"{"state_version":"24.11","system_package_count":10,"failed_assertions":["fixture failure"]}"#.into())
    }).unwrap();
    assert_eq!(result.failed_assertions, vec!["fixture failure"]);
    assert!(
        nixify::draft::evaluate_modules_with(&project, &nixpkgs, &fixture(), |_| Ok(
            "not json".into()
        ))
        .is_err()
    );
}

#[test]
fn module_evaluation_requires_explicit_nixpkgs_path() {
    let result = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .args(["validate", "somewhere", "--evaluate-modules"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--nixpkgs"));
}
