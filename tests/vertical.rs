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
