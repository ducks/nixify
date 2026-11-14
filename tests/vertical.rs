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
    assert!(text.contains("    pkgs.\"git\"\n"));
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
    assert!(before.contains("    pkgs.\"git\"\n"));
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
    value.as_object_mut().unwrap().remove("hardware");
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

#[test]
fn hardware_parsers_retain_relevant_evidence_without_identifiers() {
    use nixify::hardware;
    let pci = concat!(
        "00:1f.6 \"Ethernet controller [0200]\" \"Intel [8086]\" \"Ethernet [15b8]\" -r01\n",
        "01:00.0 \"VGA compatible controller [0300]\" \"NVIDIA [10de]\" \"Graphics [1c03]\"\n",
        "00:1f.3 \"Audio device [0403]\" \"Vendor [1234]\" \"Audio [4321]\"\n"
    );
    let parsed = hardware::parse_pci(pci).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].category, "display");
    assert_eq!(parsed[1].category, "network");
    assert!(!serde_json::to_string(&parsed).unwrap().contains("00:1f.6"));
    assert!(hardware::parse_pci("malformed output").is_err());
    let blocks = hardware::parse_lsblk(r#"{"blockdevices":[{"type":"part","fstype":"ext4","size":4096,"mountpoints":["/home/private-user","/"],"uuid":"secret-uuid","name":"secret-device"}]}"#).unwrap();
    assert_eq!(blocks[0].system_mounts, vec!["/"]);
    let json = serde_json::to_string(&blocks).unwrap();
    for private in ["private-user", "secret-uuid", "secret-device"] {
        assert!(!json.contains(private));
    }
    assert!(hardware::parse_lsblk("not-json").is_err());
    let raid = hardware::parse_lsblk(r#"{"blockdevices":[{"type":"raid1","fstype":null,"size":123,"mountpoints":[]},{"type":"loop","fstype":null,"size":2,"mountpoints":[]}]}"#).unwrap();
    assert_eq!(raid.len(), 1);
    assert_eq!(raid[0].kind, "raid1");
    assert_eq!(
        hardware::parse_cpu("model name : Example CPU\nSerial : confidential\n").as_deref(),
        Some("Example CPU")
    );
}

#[test]
fn hardware_inventory_rejects_sensitive_mount_paths() {
    let mut inv = fixture();
    inv.hardware = Some(nixify::hardware::Hardware {
        evidence: vec![],
        cpu_model: None,
        uefi_visible: None,
        pci_devices: None,
        block_devices: Some(vec![nixify::hardware::BlockDevice {
            kind: "part".into(),
            filesystem: Some("ext4".into()),
            size_bytes: 1,
            system_mounts: vec!["/home/private-user".into()],
        }]),
    });
    assert!(inv.validate().is_err());
}

#[test]
fn networking_summarizes_without_retaining_addresses_or_domains() {
    use nixify::network;
    let dns = network::parse_resolver("# test\nsearch private.example\nnameserver 127.0.0.53\nnameserver 192.0.2.42\nnameserver fe80::1%secret-interface\nnameserver secret-invalid\n");
    assert_eq!(dns.loopback_nameservers, 1);
    assert_eq!(dns.other_nameservers, 2);
    assert_eq!(dns.unrecognized_nameservers, 1);
    let json = serde_json::to_string(&dns).unwrap();
    for secret in [
        "private.example",
        "127.0.0.53",
        "192.0.2.42",
        "secret-interface",
        "secret-invalid",
    ] {
        assert!(!json.contains(secret));
    }
    let root = tempfile::tempdir().unwrap();
    for (name, kind, wireless) in [
        ("lo-secret", "772", false),
        ("wifi-secret", "1", true),
        ("vpn-secret", "65534", false),
    ] {
        let dir = root.path().join(name);
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("type"), kind).unwrap();
        if wireless {
            fs::create_dir(dir.join("wireless")).unwrap();
        }
    }
    let summary = network::interface_summary(root.path()).unwrap();
    assert_eq!(summary.loopback, 1);
    assert_eq!(summary.wireless, 1);
    assert_eq!(summary.other, 1);
    let json = serde_json::to_string(&summary).unwrap();
    assert!(!json.contains("secret"));
    fs::write(root.path().join("wifi-secret/type"), "invalid").unwrap();
    assert!(network::interface_summary(root.path()).is_err());
}

#[test]
fn missing_network_field_in_old_inventories_is_supported() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value.as_object_mut().unwrap().remove("networking");
    let inventory: Inventory = serde_json::from_value(value).unwrap();
    assert!(inventory.networking.is_none());
    assert!(inventory.validate().is_ok());
}

#[test]
fn suggestions_are_unreviewed_and_preserve_foreign_status() {
    use std::collections::BTreeMap;
    let inventory = fixture();
    let results = BTreeMap::from([
        ("base-devel".to_string(), false),
        ("foreign-tool".to_string(), true),
    ]);
    let report = nixify::suggest::report_from_results(
        &inventory,
        std::path::Path::new("/nixpkgs"),
        None,
        "x86_64-linux",
        results,
    )
    .unwrap();
    assert_eq!(report.suggestions.len(), 2);
    let foreign = report
        .suggestions
        .iter()
        .find(|x| x.source_package == "foreign-tool")
        .unwrap();
    assert!(foreign.foreign && foreign.attribute_exists);
    assert_eq!(foreign.classification, "unreviewed_exact_name_match");
    assert!(report.warning.contains("No mappings were accepted"));
    assert!(nixify::suggest::report_from_results(
        &inventory,
        std::path::Path::new("/nixpkgs"),
        None,
        "x86_64-linux",
        BTreeMap::new()
    )
    .is_err());
}

#[test]
fn reviewed_mapping_is_explicit_safe_and_revalidated() {
    use nixify::mapping::{ReviewedMapping, ReviewedMappings};
    let inventory = fixture();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("draft");
    let revision = "0123456789abcdef0123456789abcdef01234567";
    let review = ReviewedMappings {
        schema_version: 1,
        nixpkgs_revision: Some(revision.into()),
        mappings: vec![ReviewedMapping {
            source_package: "foreign-tool".into(),
            nixpkgs_attribute: "htop".into(),
        }],
    };
    let decisions = mapping::reviewed_decisions(&inventory, &review, Some(revision)).unwrap();
    assert_eq!(
        decisions
            .iter()
            .find(|d| d.package == "foreign-tool")
            .unwrap()
            .status,
        Status::Reviewed
    );
    output::generate_with_review(&inventory, &dir, revision, &review).unwrap();
    let module = fs::read_to_string(dir.join("packages.nix")).unwrap();
    assert!(module.contains("pkgs.\"htop\""));
    assert!(dir.join("reviewed-mappings.json").exists());
    let (_, attrs) = nixify::draft::expected_attributes(&dir, &inventory).unwrap();
    assert!(attrs.contains(&"htop".into()));
    assert!(fs::read_to_string(dir.join("migration-report.md"))
        .unwrap()
        .contains("User-reviewed mapping"));
}

#[test]
fn reviewed_mappings_reject_injection_duplicates_dependencies_and_wrong_revision() {
    use nixify::mapping::{ReviewedMapping, ReviewedMappings};
    let inventory = fixture();
    let item = |source: &str, attr: &str| ReviewedMapping {
        source_package: source.into(),
        nixpkgs_attribute: attr.into(),
    };
    let check = |mappings, revision: Option<&str>| ReviewedMappings {
        schema_version: 1,
        nixpkgs_revision: revision.map(str::to_owned),
        mappings,
    };
    let target = "0123456789abcdef0123456789abcdef01234567";
    for review in [
        check(vec![item("git", "x\"; builtins.abort \"bad")], None),
        check(vec![item("git", "xfce..exo")], None),
        check(vec![item("git", ".xfce")], None),
        check(vec![item("git", "xfce.exo.")], None),
        check(vec![item("git", "htop"), item("git", "git")], None),
        check(vec![item("libexample", "htop")], None),
        check(vec![item("missing", "htop")], None),
        check(
            vec![item("git", "htop")],
            Some("ffffffffffffffffffffffffffffffffffffffff"),
        ),
    ] {
        assert!(mapping::reviewed_decisions(&inventory, &review, Some(target)).is_err());
    }
}

#[test]
fn cli_generate_accepts_only_explicitly_reviewed_mappings() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = fixture();
    let input = temp.path().join("inventory.json");
    let review = temp.path().join("review.json");
    let dir = temp.path().join("draft");
    fs::write(&input, serde_json::to_vec(&inventory).unwrap()).unwrap();
    fs::write(&review, r#"{"schema_version":1,"mappings":[{"source_package":"foreign-tool","nixpkgs_attribute":"htop"}]}"#).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .args([
            "generate",
            "--input",
            input.to_str().unwrap(),
            "--output",
            dir.to_str().unwrap(),
            "--reviewed-mappings",
            review.to_str().unwrap(),
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read_to_string(dir.join("packages.nix"))
        .unwrap()
        .contains("pkgs.\"htop\""));
    assert!(nixify::draft::expected_attributes(&dir, &inventory)
        .unwrap()
        .1
        .contains(&"htop".into()));
}

#[test]
fn validation_accepts_legacy_generated_package_module() {
    let inv = fixture();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("draft");
    output::generate(&inv, &dir).unwrap();
    let legacy = "# Migration draft. Curated mappings are not yet validated against a Nixpkgs revision.\n{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [\n    git\n    neovim\n  ];\n}\n";
    fs::write(dir.join("packages.nix"), legacy).unwrap();
    assert_eq!(
        nixify::draft::expected_attributes(&dir, &inv).unwrap().1,
        vec!["git", "neovim"]
    );
}

#[test]
fn curated_mappings_handle_different_names_without_promoting_dependencies_or_foreign_packages() {
    use nixify::inventory::Package;
    let package = |name: &str, explicit: bool, foreign: bool| Package {
        name: name.into(),
        version: "1".into(),
        explicitly_installed: explicit,
        foreign,
        evidence: vec![],
    };
    let make = mapping::decision(&package("make", true, false));
    assert_eq!(make.nixpkgs_attribute.as_deref(), Some("gnumake"));
    assert_eq!(
        mapping::decision(&package("tmux", false, false)).status,
        Status::Dependency
    );
    assert_eq!(
        mapping::decision(&package("tmux", true, true)).status,
        Status::Review
    );
    assert_eq!(
        mapping::decision(&package("unrecognized-package", true, false)).status,
        Status::Unmapped
    );
}

#[test]
fn old_drafts_remain_verifiable_after_curated_mappings_expand() {
    use nixify::inventory::Package;
    let mut inv = fixture();
    inv.packages.push(Package {
        name: "tmux".into(),
        version: "1".into(),
        explicitly_installed: true,
        foreign: false,
        evidence: vec![],
    });
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("draft");
    output::generate(&inv, &dir).unwrap();
    let legacy = "# Migration draft. Curated mappings are not yet validated against a Nixpkgs revision.\n{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [\n    git\n    neovim\n  ];\n}\n";
    fs::write(dir.join("packages.nix"), legacy).unwrap();
    let (_, attrs) = nixify::draft::expected_attributes(&dir, &inv).unwrap();
    assert_eq!(attrs, vec!["git", "neovim"]);
}

#[test]
fn distribution_mapping_database_requires_safe_unique_evidenced_entries() {
    let source = include_str!("../mappings/manjaro.json");
    let original: serde_json::Value = serde_json::from_str(source).unwrap();
    let db = mapping::parse_database(source).unwrap();
    assert_eq!(db.distribution, "manjaro");
    assert!(db.entries.iter().any(|entry| entry.source_package == "make"
        && entry.nixpkgs_attribute.as_deref() == Some("gnumake")));
    let mut duplicate = original.clone();
    let entry = duplicate["entries"][0].clone();
    duplicate["entries"].as_array_mut().unwrap().push(entry);
    assert!(mapping::parse_database(&duplicate.to_string()).is_err());
    let mut injection = original.clone();
    injection["entries"][0]["nixpkgs_attribute"] = serde_json::json!("evil\"; builtins.abort \"no");
    assert!(mapping::parse_database(&injection.to_string()).is_err());
    let mut no_evidence = original.clone();
    no_evidence["entries"][0]["verification"] = serde_json::Value::Null;
    assert!(mapping::parse_database(&no_evidence.to_string()).is_err());
    let mut wrong_distro = original;
    wrong_distro["distribution"] = serde_json::json!("arch");
    assert!(mapping::parse_database(&wrong_distro.to_string()).is_err());
}

#[test]
fn strong_catalog_matches_are_inferred_and_weak_matches_remain_unresolved() {
    use nixify::inventory::Package;
    let package = |name: &str, foreign: bool, explicit: bool| Package {
        name: name.into(),
        version: "1".into(),
        explicitly_installed: explicit,
        foreign,
        evidence: vec![],
    };
    let repo = mapping::decision(&package("acl", false, true));
    assert_eq!(repo.status, Status::Inferred);
    assert!(repo.reason.contains("same name and upstream homepage"));
    assert_eq!(repo.nixpkgs_attribute.as_deref(), Some("acl"));
    let aur = mapping::decision(&package("paru", true, true));
    assert_eq!(aur.status, Status::Inferred);
    assert!(aur.reason.contains("does not prove AUR provenance"));
    assert_eq!(
        mapping::decision(&package("android-studio", true, true)).status,
        Status::Review
    );
    assert_eq!(
        mapping::decision(&package("acl", false, false)).status,
        Status::Dependency
    );
    assert!(output::packages_nix(&[repo, aur]).contains("pkgs.\"acl\""));
}

#[test]
fn generated_inferences_are_snapshotted_and_reject_unsafe_edits() {
    use nixify::inventory::Package;
    let mut inv = fixture();
    inv.packages.push(Package {
        name: "acl".into(),
        version: "1".into(),
        explicitly_installed: true,
        foreign: false,
        evidence: vec![],
    });
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("draft");
    output::generate(&inv, &dir).unwrap();
    let report = fs::read_to_string(dir.join("migration-report.md")).unwrap();
    assert!(report.contains("Inferred mappings"));
    assert!(nixify::draft::expected_attributes(&dir, &inv)
        .unwrap()
        .1
        .contains(&"acl".to_owned()));
    let path = dir.join("package-decisions.json");
    let mut decisions: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let entry = decisions
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d["package"] == "acl")
        .unwrap();
    entry["nixpkgs_attribute"] = serde_json::json!("evil\"; builtins.abort \"bad");
    fs::write(path, decisions.to_string()).unwrap();
    assert!(nixify::draft::expected_attributes(&dir, &inv).is_err());
}

#[test]
fn same_name_collision_uses_correct_nested_attribute() {
    use nixify::inventory::Package;
    let package = |name: &str| Package {
        name: name.into(),
        version: "1".into(),
        explicitly_installed: true,
        foreign: false,
        evidence: vec![],
    };
    let exo = mapping::decision(&package("exo"));
    assert_eq!(exo.status, Status::Mapped);
    assert_eq!(exo.nixpkgs_attribute.as_deref(), Some("xfce.exo"));
    assert!(output::packages_nix(&[exo]).contains("pkgs.\"xfce\".\"exo\""));
    let zola = mapping::decision(&package("zola"));
    assert_eq!(zola.status, Status::Mapped);
    assert_eq!(zola.nixpkgs_attribute.as_deref(), Some("zola"));
}

#[test]
fn drafts_from_before_catalog_expansion_remain_checkable() {
    use nixify::inventory::Package;
    let mut inv = fixture();
    inv.packages.push(Package {
        name: "zola".into(),
        version: "1".into(),
        explicitly_installed: true,
        foreign: false,
        evidence: vec![],
    });
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("draft");
    output::generate(&inv, &dir).unwrap();
    fs::remove_file(dir.join("package-decisions.json")).unwrap();
    let mut previous = mapping::decisions(&inv);
    previous
        .iter_mut()
        .find(|d| d.package == "zola")
        .unwrap()
        .nixpkgs_attribute = None;
    fs::write(dir.join("packages.nix"), output::packages_nix(&previous)).unwrap();
    let (_, attributes) = nixify::draft::expected_attributes(&dir, &inv).unwrap();
    assert!(!attributes.contains(&"zola".to_owned()));
}

#[test]
fn reviewed_nested_attributes_are_quoted_and_snapshotted() {
    use nixify::mapping::{ReviewedMapping, ReviewedMappings};
    let inv = fixture();
    let review = ReviewedMappings {
        schema_version: 1,
        nixpkgs_revision: None,
        mappings: vec![ReviewedMapping {
            source_package: "git".into(),
            nixpkgs_attribute: "xfce.exo".into(),
        }],
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("draft");
    output::generate_with_review(
        &inv,
        &dir,
        &nixify::draft::default_revision().unwrap(),
        &review,
    )
    .unwrap();
    assert!(fs::read_to_string(dir.join("packages.nix"))
        .unwrap()
        .contains("pkgs.\"xfce\".\"exo\""));
    assert!(nixify::draft::expected_attributes(&dir, &inv)
        .unwrap()
        .1
        .contains(&"xfce.exo".to_owned()));
}
