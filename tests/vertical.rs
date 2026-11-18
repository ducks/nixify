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
    let report = output::report(&inv, &mapping::decisions(&inv));
    assert!(report.contains("this draft does not enable unfree packages"));
    assert!(report.contains("checks do not establish buildability"));
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
fn report_handles_a_closed_stdout_pipe() {
    use std::process::Stdio;

    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("inventory.json");
    fs::write(&file, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .arg("report")
        .arg("--input")
        .arg(&file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
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

#[test]
fn desktop_and_workstation_services_require_persistent_system_enablement() {
    use nixify::{inventory::Scope, service_mapping, systemd};
    let mut inv = fixture();
    inv.service_scopes = vec![
        systemd::from_outputs(Scope::System, Some("avahi-daemon.service enabled enabled\nbluetooth.service enabled enabled\ncups.service enabled enabled\nlibvirtd.service enabled enabled\ntlp.service enabled enabled\nollama.service disabled disabled\nmullvad-daemon.service enabled enabled\n"), Some("ollama.service loaded active running Ollama\n")),
        systemd::from_outputs(Scope::User, Some("ollama.service enabled enabled\n"), None),
    ];
    let decisions = service_mapping::decisions(&inv);
    let module = output::services_nix(&decisions);
    for option in [
        "services.avahi.enable",
        "hardware.bluetooth.enable",
        "services.printing.enable",
        "virtualisation.libvirtd.enable",
        "services.tlp.enable",
        "services.mullvad-vpn.enable",
    ] {
        assert!(
            module.contains(&format!("{option} = true;")),
            "missing {option}"
        );
    }
    assert!(!module.contains("services.ollama.enable"));
    assert!(decisions
        .iter()
        .find(|d| d.service == "mullvad-daemon.service")
        .unwrap()
        .reason
        .contains("firewall"));
}

#[test]
fn cli_desktop_choice_is_explicit_and_preserves_other_hints() {
    use nixify::{desktop, inventory::Scope, systemd};
    let mut inv = fixture();
    inv.service_scopes = vec![systemd::from_outputs(
        Scope::System,
        Some("lemurs.service enabled enabled\n"),
        None,
    )];
    assert_eq!(desktop::enabled_display_managers(&inv), vec!["Lemurs"]);
    let root = tempfile::tempdir().unwrap();
    inv.desktop = Some(desktop::detect(
        &inv,
        Some("XFCE"),
        Some("x11"),
        root.path(),
    ));
    let inventory_path = root.path().join("inventory.json");
    fs::write(&inventory_path, serde_json::to_vec(&inv).unwrap()).unwrap();
    let dir = root.path().join("draft");
    let result = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .args(["generate", "--input"])
        .arg(&inventory_path)
        .args(["--output"])
        .arg(&dir)
        .args(["--desktop", "hyprland"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let module = fs::read_to_string(dir.join("desktop.nix")).unwrap();
    assert!(module.contains("  programs.hyprland.enable = true; # explicitly selected"));
    assert!(module.contains("  # services.xserver.desktopManager.xfce.enable = true;"));
    let report = fs::read_to_string(dir.join("migration-report.md")).unwrap();
    assert!(report.contains("Explicit desktop selection: Hyprland"));
    assert!(report.contains("Lemurs"));
}

#[test]
fn configuration_discovery_is_fixed_location_and_never_reads_contents() {
    use nixify::{
        config_evidence::{self, ConfigId, ConfigState},
        inventory::Package,
    };
    use std::os::unix::fs::symlink;
    let mut inv = fixture();
    for name in ["hyprland", "waybar", "lemurs"] {
        inv.packages.push(Package {
            name: name.into(),
            version: "1".into(),
            explicitly_installed: true,
            foreign: false,
            evidence: vec![],
        });
    }
    let root = tempfile::tempdir().unwrap();
    let sys = root.path().join("sysroot");
    let home = root.path().join("home");
    fs::create_dir_all(sys.join("etc/lemurs")).unwrap();
    fs::create_dir_all(home.join(".config/hypr")).unwrap();
    fs::write(
        sys.join("etc/lemurs/config.toml"),
        "password=secret-test-value",
    )
    .unwrap();
    fs::write(
        home.join(".config/hypr/hyprland.conf"),
        "token=secret-test-value",
    )
    .unwrap();
    symlink(home.join(".config/hypr"), home.join(".config/waybar")).unwrap();
    let entries = config_evidence::scan_at(&inv, &sys, Some(&home));
    let state = |id| entries.iter().find(|e| e.id == id).unwrap().state;
    assert_eq!(state(ConfigId::Lemurs), ConfigState::Present);
    assert_eq!(state(ConfigId::Hyprland), ConfigState::Present);
    assert_eq!(state(ConfigId::Waybar), ConfigState::SymlinkNotFollowed);
    assert!(!serde_json::to_string(&entries)
        .unwrap()
        .contains("secret-test-value"));
    assert!(!serde_json::to_string(&entries)
        .unwrap()
        .contains(root.path().to_str().unwrap()));
    inv.config_evidence = Some(entries);
    inv.validate().unwrap();
    let report = output::report(&inv, &mapping::decisions(&inv));
    assert!(report.contains("$HOME/.config/hypr/hyprland.conf"));
    assert!(report.contains("symlink encountered"));
    assert!(!report.contains("secret-test-value"));
    let empty_home = config_evidence::scan_at(&inv, &sys, None);
    assert_eq!(
        empty_home
            .iter()
            .find(|e| e.id == ConfigId::Hyprland)
            .unwrap()
            .state,
        ConfigState::Unavailable
    );
}

#[test]
fn old_inventory_without_config_evidence_remains_valid() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value.as_object_mut().unwrap().remove("config_evidence");
    let inventory: Inventory = serde_json::from_value(value).unwrap();
    assert!(inventory.config_evidence.is_none());
    inventory.validate().unwrap();
}

#[test]
fn config_profiles_retain_only_allowlisted_typed_settings() {
    use nixify::config_inspection::{self as config, Profile, Setting};
    let source = "general {\n layout = dwindle\n}\ninput {\n natural_scroll = true\n touchpad {\n disable_while_typing = true\n }\n}\nenv = TOKEN,do-not-save\nexec-once = private-command\n";
    let hypr = config::inspect(Profile::Hyprland, source).unwrap();
    assert!(matches!(
        hypr.settings.as_slice(),
        [
            Setting::HyprlandLayout(_),
            Setting::HyprlandNaturalScroll(true)
        ]
    ));
    assert_eq!(hypr.unhandled_items, 3);
    let encoded = serde_json::to_string(&hypr).unwrap();
    assert!(!encoded.contains("do-not-save") && !encoded.contains("private-command"));
    let overridden = config::inspect(
        Profile::Hyprland,
        "source = private.conf\ngeneral {\nlayout = master\n}\n",
    )
    .unwrap();
    assert!(overridden.incomplete && overridden.settings.is_empty());
    let duplicate = config::inspect(
        Profile::Hyprland,
        "input {\nnatural_scroll = true\nnatural_scroll = false\n}\n",
    )
    .unwrap();
    assert!(duplicate.incomplete && duplicate.settings.is_empty());
    let docker = config::inspect(
        Profile::DockerDaemon,
        r#"{"live-restore":true,"auths":{"secret":"hidden"}}"#,
    )
    .unwrap();
    assert!(matches!(
        docker.settings.as_slice(),
        [Setting::DockerLiveRestore(true)]
    ));
    assert_eq!(docker.unhandled_items, 1);
    assert!(!serde_json::to_string(&docker).unwrap().contains("hidden"));
    assert!(config::inspect(
        Profile::DockerDaemon,
        r#"{"live-restore":true,"live-restore":false}"#
    )
    .is_err());
    assert!(config::inspect(Profile::DockerDaemon, r#"{"live-restore":"true"}"#).is_err());
}

#[test]
fn inspected_docker_setting_requires_explicit_file_and_enabled_service() {
    use nixify::{
        config_inspection::{self as config, Profile},
        inventory::Scope,
        systemd,
    };
    let mut inv = fixture();
    let inspection = config::inspect(
        Profile::DockerDaemon,
        r#"{"live-restore":true,"registry-token":"dont-copy"}"#,
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let review = mapping::ReviewedMappings::empty();
    let revision = nixify::draft::default_revision().unwrap();
    let disabled = root.path().join("disabled");
    output::generate_with_inspections(
        &inv,
        &disabled,
        &revision,
        &review,
        None,
        std::slice::from_ref(&inspection),
    )
    .unwrap();
    assert!(!fs::read_to_string(disabled.join("services.nix"))
        .unwrap()
        .contains("live-restore"));
    inv.service_scopes = vec![systemd::from_outputs(
        Scope::System,
        Some("docker.service enabled enabled\n"),
        None,
    )];
    let enabled = root.path().join("enabled");
    output::generate_with_inspections(&inv, &enabled, &revision, &review, None, &[inspection])
        .unwrap();
    let module = fs::read_to_string(enabled.join("services.nix")).unwrap();
    assert!(module.contains("virtualisation.docker.daemon.settings.\"live-restore\" = true;"));
    assert!(!module.contains("dont-copy"));
    assert!(
        !fs::read_to_string(enabled.join("configuration-inspections.json"))
            .unwrap()
            .contains("dont-copy")
    );
}

#[test]
fn explicit_config_read_rejects_symlinks_and_oversized_files() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    fs::write(&file, vec![b'x'; 256 * 1024 + 1]).unwrap();
    assert!(nixify::config_inspection::read_explicit(&file).is_err());
    let link = dir.path().join("link");
    symlink(&file, &link).unwrap();
    assert!(nixify::config_inspection::read_explicit(&link).is_err());
}

#[test]
fn cli_inspects_config_and_generates_draft_without_raw_values() {
    use nixify::{inventory::Scope, systemd};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("daemon.json");
    let inspection = temp.path().join("inspection.json");
    fs::write(
        &source,
        r#"{"live-restore":false,"secret":"do-not-export"}"#,
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .args(["inspect-config", "--profile", "docker-daemon", "--file"])
        .arg(&source)
        .args(["--output"])
        .arg(&inspection)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!fs::read_to_string(&inspection)
        .unwrap()
        .contains("do-not-export"));
    let mut inv = fixture();
    inv.service_scopes = vec![systemd::from_outputs(
        Scope::System,
        Some("docker.service enabled enabled\n"),
        None,
    )];
    let inventory = temp.path().join("inventory.json");
    fs::write(&inventory, serde_json::to_vec(&inv).unwrap()).unwrap();
    let dir = temp.path().join("draft");
    let result = Command::new(env!("CARGO_BIN_EXE_nixify"))
        .args(["generate", "--input"])
        .arg(&inventory)
        .args(["--output"])
        .arg(&dir)
        .args(["--config-inspection"])
        .arg(&inspection)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read_to_string(dir.join("services.nix"))
        .unwrap()
        .contains("daemon.settings.\"live-restore\" = false;"));
    assert!(!fs::read_to_string(dir.join("migration-report.md"))
        .unwrap()
        .contains("do-not-export"));
}

#[test]
fn catalog_suggestions_are_ranked_unreviewed_and_handle_nested_names() {
    use nixify::{catalog, inventory::Package};
    let mut inv = fixture();
    for (name, foreign) in [("helix-bin", true), ("mystery", false)] {
        inv.packages.push(Package {
            name: name.into(),
            version: "1".into(),
            explicitly_installed: true,
            foreign,
            evidence: vec![],
        });
    }
    let catalog_data = serde_json::json!({
        "editors.helix": {"pname":"helix", "version":"1", "meta":{"description":"A text editor"}},
        "tools.hidden": {"pname":"mystery", "version":"2", "meta":{"description":"Different attribute name"}},
        "others.mystery": {"pname":"other", "version":"3"},
        "evil\";abort": {"pname":"mystery"}
    });
    let report = catalog::from_value(
        &inv,
        &catalog_data,
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let helix = report
        .matches
        .iter()
        .find(|m| m.source_package == "helix-bin")
        .unwrap();
    assert_eq!(helix.candidates[0].attribute, "editors.helix");
    assert_eq!(helix.candidates[0].match_kind, "foreign_suffix_removed");
    let mystery = report
        .matches
        .iter()
        .find(|m| m.source_package == "mystery")
        .unwrap();
    assert_eq!(mystery.candidates.len(), 2);
    assert_eq!(mystery.candidates[0].attribute, "others.mystery");
    assert_eq!(mystery.candidates[1].match_kind, "package_name");
    assert!(report.warning.contains("not equivalence"));
    assert!(catalog::from_value(&inv, &catalog_data, "invalid").is_err());
}

#[test]
fn catalog_cli_reads_offline_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let inv = root.path().join("inventory.json");
    let catalog = root.path().join("packages.json");
    let out = root.path().join("matches.json");
    fs::write(&inv, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    fs::write(&catalog, r#"{"packages":{"tools.foreign-tool":{"pname":"foreign-tool","version":"1","meta":{"description":"Example"}}},"version":"fixture"}"#).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_nixify"))
            .args(["catalog-suggest", "--input"])
            .arg(&inv)
            .args(["--catalog"])
            .arg(&catalog)
            .args([
                "--revision",
                "0123456789abcdef0123456789abcdef01234567",
                "--output",
            ])
            .arg(&out)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let data = fs::read_to_string(&out).unwrap();
    assert!(data.contains("tools.foreign-tool"));
    assert!(!run().status.success());
    assert_eq!(fs::read_to_string(&out).unwrap(), data);
}

#[test]
fn channel_catalog_rejects_bad_wrapper_and_trailing_data() {
    use nixify::catalog;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("packages.json");
    let revision = "0123456789abcdef0123456789abcdef01234567";
    for text in [
        r#"{"version":"1"}"#,
        r#"{"packages":{}} trailing"#,
        r#"{"packages":{},"packages":{}}"#,
    ] {
        fs::write(&file, text).unwrap();
        assert!(catalog::read(&fixture(), &file, revision).is_err());
    }
}

#[test]
fn catalog_suggests_distribution_name_variants_without_promoting_them() {
    use nixify::inventory::Package;
    let mut inv = fixture();
    for name in ["grep", "python-pipx", "xorg-xinit", "procps-ng"] {
        inv.packages.push(Package {
            name: name.into(),
            version: "1".into(),
            explicitly_installed: true,
            foreign: false,
            evidence: vec![],
        });
    }
    let data = serde_json::json!({"packages": {
        "gnugrep": {"pname":"gnugrep"}, "pipx": {"pname":"pipx"},
        "xinit": {"pname":"xinit"}, "procps": {"pname":"procps"}
    }});
    let report =
        nixify::catalog::from_value(&inv, &data, "0123456789abcdef0123456789abcdef01234567")
            .unwrap();
    for (name, attribute, kind) in [
        ("grep", "gnugrep", "gnu_prefix_added"),
        ("python-pipx", "pipx", "distribution_prefix_removed"),
        ("xorg-xinit", "xinit", "distribution_prefix_removed"),
        ("procps-ng", "procps", "known_name_variant"),
    ] {
        let entry = report
            .matches
            .iter()
            .find(|entry| entry.source_package == name)
            .unwrap();
        assert!(entry
            .candidates
            .iter()
            .any(|candidate| candidate.attribute == attribute && candidate.match_kind == kind));
        assert_eq!(
            mapping::decision(inv.packages.iter().find(|p| p.name == name).unwrap()).status,
            Status::Unmapped
        );
    }
}

#[test]
fn hardware_generator_uses_show_mode_and_excludes_filesystems_by_default() {
    let config = nixify::hardware::generate_config_with(false, |program, args| {
        assert_eq!(program, "nixos-generate-config");
        assert_eq!(args, ["--show-hardware-config", "--no-filesystems"]);
        Ok("{ ... }: {}\n".into())
    })
    .unwrap();
    assert!(config.contains("{ ... }"));
    nixify::hardware::generate_config_with(true, |_, args| {
        assert_eq!(args, ["--show-hardware-config"]);
        Ok("{}".into())
    })
    .unwrap();
    assert!(nixify::hardware::generate_config_with(false, |_, _| Ok(String::new())).is_err());
}

#[test]
fn hardware_config_cli_captures_stdout_without_overwriting() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let fake = root.path().join("nixos-generate-config");
    fs::write(&fake, "#!/bin/sh\n[ \"$1\" = --show-hardware-config ] && [ \"$2\" = --no-filesystems ] || exit 2\nprintf '{ ... }: {}\\n'\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let target = root.path().join("hardware.nix");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_nixify"))
            .args(["hardware-config", "--output"])
            .arg(&target)
            .env("PATH", root.path())
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "{ ... }: {}\n");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!run().status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), "{ ... }: {}\n");
}

#[test]
fn verified_coverage_is_default_revision_scoped_and_auditable() {
    use nixify::{desktop::DesktopKind, draft, inventory::Scope, systemd};
    let mut inventory = manjaro::inventory(
        "ID=manjaro",
        "x86_64",
        "bzip2 1\ndocker 1\nhyprland 1\nnetworkmanager 1\nfirefox 1\n",
        "bzip2\ndocker\nhyprland\nnetworkmanager\nfirefox\n",
        "",
    )
    .unwrap();
    inventory.service_scopes = vec![systemd::from_outputs(
        Scope::System,
        Some("docker.service enabled enabled\nNetworkManager.service enabled enabled\n"),
        None,
    )];
    let revision = "eeeb90a1dd3c9bea3afdbc76fd34d0fb2a727c7a";
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("draft");
    output::generate_with_desktop(
        &inventory,
        &dest,
        revision,
        &mapping::ReviewedMappings::empty(),
        Some(DesktopKind::Hyprland),
    )
    .unwrap();
    let packages = fs::read_to_string(dest.join("packages.nix")).unwrap();
    let decisions: Vec<mapping::Decision> =
        serde_json::from_slice(&fs::read(dest.join("package-decisions.json")).unwrap()).unwrap();
    for name in ["docker", "hyprland", "networkmanager"] {
        let decision = decisions.iter().find(|d| d.package == name).unwrap();
        assert!(decision.covered_by.is_some(), "{name}: {decision:?}");
        assert!(!packages.contains(&format!("pkgs.\"{name}\"")));
    }
    assert!(packages.contains("pkgs.\"bzip2\""));
    assert!(packages.contains("pkgs.\"firefox\""));
    let report = fs::read_to_string(dest.join("migration-report.md")).unwrap();
    assert!(report.contains("Direct package entries omitted"));
    assert!(report.contains("programs.hyprland.enable"));
    let (_, attrs) = draft::expected_attributes(&dest, &inventory).unwrap();
    assert!(attrs.contains(&"firefox".to_owned()));
    assert!(!attrs.contains(&"docker".to_owned()));
    let mut stale = decisions.clone();
    stale
        .iter_mut()
        .find(|d| d.package == "bzip2")
        .unwrap()
        .covered_by = Some(nixify::coverage::Coverage {
        provider: "NixOS required system packages".into(),
        nixpkgs_revision: revision.into(),
    });
    fs::write(dest.join("packages.nix"), output::packages_nix(&stale)).unwrap();
    fs::write(
        dest.join("package-decisions.json"),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    assert!(draft::expected_attributes(&dest, &inventory).is_err());
    fs::write(dest.join("packages.nix"), &packages).unwrap();
    fs::write(
        dest.join("package-decisions.json"),
        serde_json::to_vec(&decisions).unwrap(),
    )
    .unwrap();
    let services = fs::read_to_string(dest.join("services.nix")).unwrap();
    fs::write(
        dest.join("services.nix"),
        services.replace("virtualisation.docker.enable = true;", "# disabled"),
    )
    .unwrap();
    assert!(draft::expected_attributes(&dest, &inventory)
        .unwrap_err()
        .contains("services.nix was edited"));
    fs::write(dest.join("services.nix"), services).unwrap();
    let desktop = fs::read_to_string(dest.join("desktop.nix")).unwrap();
    fs::write(
        dest.join("desktop.nix"),
        desktop.replace("programs.hyprland.enable = true;", "# disabled"),
    )
    .unwrap();
    assert!(draft::expected_attributes(&dest, &inventory)
        .unwrap_err()
        .contains("desktop.nix was edited"));

    let inspection = nixify::config_inspection::inspect(
        nixify::config_inspection::Profile::DockerDaemon,
        r#"{"live-restore":true}"#,
    )
    .unwrap();
    let inspected = temp.path().join("inspected");
    output::generate_with_inspections(
        &inventory,
        &inspected,
        revision,
        &mapping::ReviewedMappings::empty(),
        Some(DesktopKind::Hyprland),
        &[inspection],
    )
    .unwrap();
    assert!(draft::expected_attributes(&inspected, &inventory).is_ok());

    let mut no_evidence = mapping::decisions(&inventory);
    nixify::coverage::apply(
        &inventory,
        &mut no_evidence,
        &"0".repeat(40),
        Some(DesktopKind::Hyprland),
    );
    assert!(no_evidence.iter().all(|d| d.covered_by.is_none()));
    inventory.source.architecture = "aarch64".into();
    nixify::coverage::apply(
        &inventory,
        &mut no_evidence,
        revision,
        Some(DesktopKind::Hyprland),
    );
    assert!(no_evidence.iter().all(|d| d.covered_by.is_none()));
}

#[test]
fn coverage_does_not_override_review_foreign_or_runtime_only_services() {
    use nixify::{desktop::DesktopKind, inventory::Scope, systemd};
    let mut inventory = manjaro::inventory(
        "ID=manjaro",
        "x86_64",
        "bzip2 1\ndocker 1\nhyprland 1\nnetworkmanager 1\n",
        "bzip2\ndocker\nhyprland\nnetworkmanager\n",
        "bzip2\n",
    )
    .unwrap();
    inventory.service_scopes = vec![systemd::from_outputs(
        Scope::System,
        Some("docker.service enabled-runtime enabled\nNetworkManager.service enabled enabled\n"),
        None,
    )];
    let review = mapping::ReviewedMappings {
        schema_version: 1,
        nixpkgs_revision: None,
        mappings: vec![mapping::ReviewedMapping {
            source_package: "networkmanager".into(),
            nixpkgs_attribute: "networkmanager".into(),
        }],
    };
    let revision = "9fbb54b33e91ee4ca368e35a78e0613c720600b3";
    let mut decisions = mapping::reviewed_decisions(&inventory, &review, Some(revision)).unwrap();
    nixify::coverage::apply(&inventory, &mut decisions, revision, None);
    for name in ["bzip2", "docker", "hyprland", "networkmanager"] {
        assert!(
            decisions
                .iter()
                .find(|d| d.package == name)
                .unwrap()
                .covered_by
                .is_none(),
            "{name}"
        );
    }
    // The newer revision's Hyprland override is not verified offline.
    nixify::coverage::apply(
        &inventory,
        &mut decisions,
        revision,
        Some(DesktopKind::Hyprland),
    );
    assert!(decisions
        .iter()
        .find(|d| d.package == "hyprland")
        .unwrap()
        .covered_by
        .is_none());
}

#[test]
fn package_groups_are_generic_review_hints_not_install_decisions() {
    let mut inventory = fixture();
    let groups = nixify::package_groups::parse(
        "development git\nshared neovim\ndevelopment neovim\n",
        &inventory,
    )
    .unwrap();
    assert_eq!(groups.len(), 2);
    inventory.package_groups = Some(groups);
    inventory.validate().unwrap();
    let decisions = mapping::decisions(&inventory);
    let before = output::packages_nix(&decisions);
    let report = output::report(&inventory, &decisions);
    assert!(report.contains("### `development` — 2 explicit"));
    assert!(report.contains("Groups may overlap"));
    assert!(report.contains("do not change the generated configuration"));
    assert_eq!(
        before,
        output::packages_nix(&mapping::decisions(&inventory))
    );
    assert!(nixify::package_groups::parse("development no-such-package\n", &inventory).is_err());
    assert!(
        nixify::package_groups::parse("development git\ndevelopment git\n", &inventory).is_err()
    );
    assert!(nixify::package_groups::parse("bad;group git\n", &inventory).is_err());
    assert!(nixify::package_groups::parse("development git extra\n", &inventory).is_err());
    let mut old = serde_json::to_value(&inventory).unwrap();
    old.as_object_mut().unwrap().remove("package_groups");
    assert!(serde_json::from_value::<Inventory>(old)
        .unwrap()
        .package_groups
        .is_none());
}

#[test]
fn target_coverage_compares_generated_modules_and_persists_reviewable_evidence() {
    use nixify::{coverage_eval, draft};
    use std::collections::BTreeMap;
    let inventory = fixture();
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("checkout");
    fs::create_dir(&checkout).unwrap();
    let revision = "0123456789abcdef0123456789abcdef01234567";
    fs::write(checkout.join(".git-revision"), revision).unwrap();
    let evidence = coverage_eval::evaluate_with(&inventory, revision, &mapping::ReviewedMappings::empty(), None, &[], &checkout, |args| {
        let argument = |name: &str| -> &str {
            let index = args.windows(2).position(|x| x == ["--argstr", name]).unwrap();
            args[index + 2]
        };
        let project = std::path::Path::new(argument("projectPath"));
        assert_eq!(fs::read_to_string(project.join("packages.nix")).unwrap(), output::packages_nix(&[]));
        let attrs: Vec<String> = serde_json::from_slice(&fs::read(argument("attributesPath")).unwrap()).unwrap();
        let results: BTreeMap<_, _> = attrs.into_iter().map(|name| {
            let result = if name == "git" {
                serde_json::json!({"state":"matched", "evidence":{"path":"/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-git", "outputs":["/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-git"], "priority":5}})
            } else {
                serde_json::json!({"state":"unavailable", "evidence":null})
            };
            (name, result)
        }).collect();
        Ok(serde_json::json!({"results":results,"unavailable_system_packages":1,"failed_assertions":[]}).to_string())
    }).unwrap();
    assert_eq!(evidence.matched.len(), 1);
    assert!(evidence.unavailable_candidates > 0);
    let dir = root.path().join("draft");
    output::generate_with_coverage(
        &inventory,
        &dir,
        revision,
        &mapping::ReviewedMappings::empty(),
        None,
        &[],
        Some(&evidence),
    )
    .unwrap();
    assert!(!fs::read_to_string(dir.join("packages.nix"))
        .unwrap()
        .contains("pkgs.\"git\""));
    assert!(fs::read_to_string(dir.join("migration-report.md"))
        .unwrap()
        .contains("evaluated target system packages"));
    let (_, attrs) = draft::expected_attributes(&dir, &inventory).unwrap();
    assert!(!attrs.contains(&"git".to_string()));
    fs::remove_file(dir.join("coverage-evaluation.json")).unwrap();
    assert!(draft::expected_attributes(&dir, &inventory).is_err());
    let mut empty = evidence.clone();
    empty.matched.clear();
    let empty_dir = root.path().join("empty-matches");
    output::generate_with_coverage(
        &inventory,
        &empty_dir,
        revision,
        &mapping::ReviewedMappings::empty(),
        None,
        &[],
        Some(&empty),
    )
    .unwrap();
    assert!(draft::expected_attributes(&empty_dir, &inventory).is_ok());
    empty.schema_version = 99;
    fs::write(
        empty_dir.join("coverage-evaluation.json"),
        serde_json::to_vec(&empty).unwrap(),
    )
    .unwrap();
    assert!(draft::expected_attributes(&empty_dir, &inventory).is_err());
}

#[test]
fn target_coverage_refuses_wrong_revision_and_incomplete_results() {
    use nixify::coverage_eval;
    let inventory = fixture();
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join(".git-revision"),
        "ffffffffffffffffffffffffffffffffffffffff",
    )
    .unwrap();
    let revision = "0123456789abcdef0123456789abcdef01234567";
    assert!(coverage_eval::evaluate_with(
        &inventory,
        revision,
        &mapping::ReviewedMappings::empty(),
        None,
        &[],
        root.path(),
        |_| panic!("must not evaluate mismatched checkout")
    )
    .unwrap_err()
    .contains("does not match"));
    fs::write(root.path().join(".git-revision"), revision).unwrap();
    let error = coverage_eval::evaluate_with(
        &inventory,
        revision,
        &mapping::ReviewedMappings::empty(),
        None,
        &[],
        root.path(),
        |_| {
            Ok(
                r#"{"results":{},"unavailable_system_packages":0,"failed_assertions":[]}"#
                    .to_owned(),
            )
        },
    )
    .unwrap_err();
    assert!(error.contains("incomplete"));
}

#[test]
fn evaluated_coverage_keeps_foreign_and_explicitly_reviewed_packages() {
    use nixify::coverage_eval::{self, Evidence, MatchEvidence};
    let mut inventory = fixture();
    let revision = "0123456789abcdef0123456789abcdef01234567";
    let review = mapping::ReviewedMappings {
        schema_version: 1,
        nixpkgs_revision: None,
        mappings: vec![mapping::ReviewedMapping {
            source_package: "git".into(),
            nixpkgs_attribute: "git".into(),
        }],
    };
    let mut decisions = mapping::reviewed_decisions(&inventory, &review, Some(revision)).unwrap();
    let count = decisions
        .iter()
        .filter_map(|d| d.nixpkgs_attribute.as_ref())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let evidence = Evidence {
        schema_version: 1,
        nixpkgs_revision: revision.into(),
        system: "x86_64-linux".into(),
        matched: [(
            "git".into(),
            MatchEvidence {
                path: "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-git".into(),
                outputs: vec!["/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-git".into()],
                priority: 5,
            },
        )]
        .into_iter()
        .collect(),
        candidate_count: count,
        unavailable_candidates: 0,
        unavailable_system_packages: 0,
    };
    coverage_eval::apply(&inventory, &mut decisions, &evidence, revision).unwrap();
    assert!(decisions
        .iter()
        .find(|d| d.package == "git")
        .unwrap()
        .covered_by
        .is_none());
    let mut decisions = mapping::decisions(&inventory);
    inventory
        .packages
        .iter_mut()
        .find(|p| p.name == "git")
        .unwrap()
        .foreign = true;
    coverage_eval::apply(&inventory, &mut decisions, &evidence, revision).unwrap();
    assert!(decisions
        .iter()
        .find(|d| d.package == "git")
        .unwrap()
        .covered_by
        .is_none());
}

#[test]
#[ignore = "requires nix-instantiate; run explicitly for target coverage integration"]
fn nix_target_coverage_respects_outputs_and_link_priority() {
    use nixify::coverage_eval;
    let inventory = manjaro::inventory(
        "ID=manjaro",
        "x86_64",
        "git 1\nneovim 1\nbzip2 1\n",
        "git\nneovim\nbzip2\n",
        "",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("nixpkgs");
    fs::create_dir_all(checkout.join("nixos/lib")).unwrap();
    let revision = "0123456789abcdef0123456789abcdef01234567";
    fs::write(checkout.join(".git-revision"), revision).unwrap();
    fs::write(checkout.join("nixos/lib/eval-config.nix"), r#"
{ system, modules }:
let
  mk = name: priority: outputs:
    let path = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-" + name;
    in { outPath = path; out.outPath = path; bin.outPath = path + "-bin"; meta = { inherit priority; outputsToInstall = outputs; }; };
  git = mk "git" 5 [ "out" ];
  neovim = mk "neovim" 5 [ "out" ];
  bzip2 = mk "bzip2" 5 [ "out" "bin" ];
  lib = {
    splitString = separator: value: [ value ];
    attrByPath = parts: fallback: attrs: if builtins.hasAttr (builtins.head parts) attrs then attrs.${builtins.head parts} else fallback;
    meta.defaultPriority = 5;
  };
in {
  pkgs = { inherit lib git neovim bzip2; };
  config = {
    environment = { systemPackages = [ git (mk "neovim" 8 [ "out" ]) (mk "bzip2" 5 [ "out" ]) ]; extraOutputsToInstall = []; };
    assertions = [];
  };
}
"#).unwrap();
    let evidence = coverage_eval::evaluate(
        &inventory,
        revision,
        &mapping::ReviewedMappings::empty(),
        None,
        &[],
        &checkout,
    )
    .unwrap();
    assert_eq!(
        evidence
            .matched
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["git"]
    );
    assert_eq!(evidence.unavailable_candidates, 0);
}
