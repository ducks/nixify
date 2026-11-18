use clap::{Parser, Subcommand, ValueEnum};
use nixify::{
    baseline, catalog, config_inspection, desktop::DesktopKind, draft, inventory::Inventory,
    manjaro, mapping, output, suggest,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "Inspect a Linux system and prepare a reviewable NixOS migration draft"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Clone, Copy, ValueEnum)]
enum DesktopSelection {
    Hyprland,
    Xfce,
}
#[derive(Clone, Copy, ValueEnum)]
enum ConfigProfile {
    Hyprland,
    DockerDaemon,
}
impl From<ConfigProfile> for config_inspection::Profile {
    fn from(value: ConfigProfile) -> Self {
        match value {
            ConfigProfile::Hyprland => Self::Hyprland,
            ConfigProfile::DockerDaemon => Self::DockerDaemon,
        }
    }
}
impl From<DesktopSelection> for DesktopKind {
    fn from(value: DesktopSelection) -> Self {
        match value {
            DesktopSelection::Hyprland => Self::Hyprland,
            DesktopSelection::Xfce => Self::Xfce,
        }
    }
}
#[derive(Subcommand)]
enum Command {
    /// Read-only Manjaro package inventory. No Nix installation is required.
    Scan {
        #[arg(long)]
        distro: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Capture hardware-only output from the installed NixOS configuration generator.
    HardwareConfig {
        #[arg(long)]
        output: PathBuf,
        /// Include currently mounted filesystems; review carefully for the intended target.
        #[arg(long)]
        include_filesystems: bool,
    },
    /// Generate a package module and migration report in a new directory.
    Generate {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Full pinned Nixpkgs Git revision (defaults to nixify's locked revision).
        #[arg(long)]
        nixpkgs_rev: Option<String>,
        /// Evaluate a trusted local checkout matching the target revision for exact package overlap.
        #[arg(long)]
        nixpkgs: Option<PathBuf>,
        /// Explicitly reviewed mappings in JSON; never inferred automatically.
        #[arg(long)]
        reviewed_mappings: Option<PathBuf>,
        /// Explicitly choose a desktop to enable; detection alone remains a hint.
        #[arg(long, value_enum)]
        desktop: Option<DesktopSelection>,
        /// Include sanitized, explicitly generated configuration inspection(s).
        #[arg(long)]
        config_inspection: Vec<PathBuf>,
    },
    /// Inspect an explicitly selected configuration file using a supported profile.
    InspectConfig {
        #[arg(long, value_enum)]
        profile: ConfigProfile,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Check Nix syntax without resolving inputs, building, or activating.
    Validate {
        directory: PathBuf,
        /// Check generated package names against this local Nixpkgs checkout.
        #[arg(long)]
        nixpkgs: Option<PathBuf>,
        /// Evaluate trusted project modules with temporary placeholder hardware settings.
        #[arg(long, requires = "nixpkgs")]
        evaluate_modules: bool,
    },
    /// Evaluate the default NixOS system packages from a local Nixpkgs checkout.
    Baseline {
        #[arg(long)]
        nixpkgs: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Look up unreviewed exact-name candidates in a trusted local Nixpkgs checkout.
    Suggest {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        nixpkgs: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Suggest matches using an existing Nixpkgs packages.json catalog.
    CatalogSuggest {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        catalog: PathBuf,
        /// Full Nixpkgs revision associated with the catalog (not authenticated).
        #[arg(long)]
        revision: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Explain curated mapping coverage and unresolved packages.
    Report {
        #[arg(long)]
        input: PathBuf,
        /// Optional name-only comparison against an evaluated NixOS baseline.
        #[arg(long)]
        baseline: Option<PathBuf>,
        /// Include explicit user-reviewed mappings in the coverage report.
        #[arg(long)]
        reviewed_mappings: Option<PathBuf>,
    },
}
fn read_inventory(path: &Path) -> Result<Inventory, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("inventory exceeds 16 MiB".into());
    }
    let inventory: Inventory = serde_json::from_slice(&bytes)?;
    inventory.validate().map_err(std::io::Error::other)?;
    Ok(inventory)
}
// Unlike std's printing macros, these propagate a closed stdout pipe as an
// ordinary I/O error so commands like `nixify report ... | head` work cleanly.
macro_rules! println {
    ($($arg:tt)*) => { writeln!(std::io::stdout().lock(), $($arg)*)? };
}
macro_rules! print {
    ($($arg:tt)*) => { write!(std::io::stdout().lock(), $($arg)*)? };
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::HardwareConfig {
            output,
            include_filesystems,
        } => {
            let config = nixify::hardware::generate_config_with(include_filesystems, |program, args| {
                manjaro::run_bounded(program, args, false).map_err(|error| {
                    format!("hardware generation failed: {error}; install nixos-generate-config from nixos-install-tools and ensure it is on PATH")
                })
            }).map_err(std::io::Error::other)?;
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            options.open(&output)?.write_all(config.as_bytes())?;
            eprintln!("Wrote hardware suggestions to {}. Review before use; filesystem settings may describe the source machine, not the intended target.", output.display());
        }
        Command::CatalogSuggest {
            input,
            catalog: source,
            revision,
            output,
        } => {
            let inventory = read_inventory(&input)?;
            let report =
                catalog::read(&inventory, &source, &revision).map_err(std::io::Error::other)?;
            let data = serde_json::to_string_pretty(&report)? + "\n";
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            options.open(&output)?.write_all(data.as_bytes())?;
            eprintln!(
                "Wrote unreviewed catalog candidates to {}",
                output.display()
            );
        }
        Command::Scan {
            distro,
            output: destination,
        } => {
            let inventory = manjaro::scan(distro.as_deref())?;
            let data = serde_json::to_string_pretty(&inventory)? + "\n";
            if let Some(path) = destination {
                #[cfg(unix)]
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                let mut f = options.open(&path)?;
                f.write_all(data.as_bytes())?;
                eprintln!("Wrote {}", path.display());
            } else {
                print!("{data}");
            }
        }
        Command::Generate {
            input,
            output: dir,
            nixpkgs_rev,
            nixpkgs,
            reviewed_mappings,
            desktop,
            config_inspection: inspection_paths,
        } => {
            let inventory = read_inventory(&input)?;
            let revision = match nixpkgs_rev {
                Some(revision) => revision,
                None => draft::default_revision().map_err(std::io::Error::other)?,
            };
            let reviewed = match reviewed_mappings {
                Some(path) => mapping::read_reviewed(&path).map_err(std::io::Error::other)?,
                None => mapping::ReviewedMappings::empty(),
            };
            let mut inspections = Vec::new();
            for path in inspection_paths {
                let text =
                    config_inspection::read_explicit(&path).map_err(std::io::Error::other)?;
                let inspection: config_inspection::Inspection = serde_json::from_str(&text)?;
                inspection.validate().map_err(std::io::Error::other)?;
                inspections.push(inspection);
            }
            let selected = desktop.map(Into::into);
            let revision = draft::normalize_revision(&revision).map_err(std::io::Error::other)?;
            let evidence = if let Some(checkout) = nixpkgs {
                if dir.try_exists()? {
                    return Err(format!("output already exists: {}", dir.display()).into());
                }
                eprintln!("Evaluating trusted Nixpkgs checkout for target package overlap...");
                Some(
                    nixify::coverage_eval::evaluate(
                        &inventory,
                        &revision,
                        &reviewed,
                        selected,
                        &inspections,
                        &checkout,
                    )
                    .map_err(std::io::Error::other)?,
                )
            } else {
                None
            };
            output::generate_with_coverage(
                &inventory,
                &dir,
                &revision,
                &reviewed,
                selected,
                &inspections,
                evidence.as_ref(),
            )?;
            eprintln!("Wrote migration draft to {}", dir.display());
        }
        Command::InspectConfig {
            profile,
            file,
            output,
        } => {
            let content = config_inspection::read_explicit(&file).map_err(std::io::Error::other)?;
            let inspection = config_inspection::inspect(profile.into(), &content)
                .map_err(std::io::Error::other)?;
            let data = serde_json::to_string_pretty(&inspection)? + "\n";
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            options.open(&output)?.write_all(data.as_bytes())?;
            eprintln!("Wrote sanitized inspection to {}", output.display());
        }
        Command::Validate {
            directory,
            nixpkgs,
            evaluate_modules,
        } => {
            draft::validate_syntax(&directory).map_err(std::io::Error::other)?;
            println!(
                "Syntax: passed ({} generated Nix files)",
                draft::NIX_FILES.len()
            );
            let mut missing = false;
            if let Some(checkout) = nixpkgs {
                println!("Nixpkgs checkout requested: {}", checkout.display());
                let inventory = read_inventory(&directory.join("inventory.json"))?;
                match draft::expected_attributes(&directory, &inventory) {
                    Ok(_) => {
                        let checked = draft::check_attributes(&directory, &inventory, &checkout)
                            .map_err(std::io::Error::other)?;
                        println!("Nixpkgs checkout: {}", checked.checkout.display());
                        println!("Target pinned revision: {}", checked.target_revision);
                        match checked.revision_evidence {
                            draft::RevisionEvidence::DeclaredMatch => println!("Revision: checkout .git-revision matches the target pin (self-reported metadata; content not authenticated)"),
                            draft::RevisionEvidence::DeclaredMismatch(rev) => println!("Revision: checkout declares {rev}; it differs from the target pin, so the target revision is not validated"),
                            draft::RevisionEvidence::Unavailable => println!("Revision: checkout revision unavailable; target pin is not validated"),
                        }
                        if checked.attributes.is_empty() {
                            println!("Package attributes: none selected");
                        }
                        for (attribute, exists) in &checked.attributes {
                            println!(
                                "Package attribute {attribute}: {}",
                                if *exists {
                                    "exists in supplied checkout"
                                } else {
                                    "missing in supplied checkout"
                                }
                            );
                            missing |= !exists;
                        }
                    }
                    Err(reason) => {
                        println!("Package attributes: not checked ({reason})");
                        println!("Revision: generated target pin was not verified");
                    }
                }
                if evaluate_modules {
                    let evaluated = draft::evaluate_modules(&directory, &checkout, &inventory)
                        .map_err(std::io::Error::other)?;
                    println!("NixOS module evaluation: selected options and {} system packages evaluated using temporary placeholder hardware (stateVersion {})", evaluated.system_package_count, evaluated.state_version);
                    if evaluated.failed_assertions.is_empty() {
                        println!("Evaluated NixOS assertions: passed");
                    } else {
                        for message in &evaluated.failed_assertions {
                            println!("Failed NixOS assertion: {message}");
                        }
                        missing = true;
                    }
                    println!("Module evaluation is partial; it does not prove target-revision, hardware, service or boot compatibility.");
                } else {
                    println!("NixOS module evaluation: not checked");
                }
            } else {
                println!("Nixpkgs attributes: not checked\nNixOS module evaluation: not checked");
            }
            println!("System build: not checked\nBoot and services: not checked");
            println!("This is an incomplete migration draft; passing these checks does not make it installable.");
            if missing {
                return Err("attribute checks or evaluated NixOS assertions failed".into());
            }
        }

        Command::Baseline { nixpkgs, output } => {
            let data = serde_json::to_string_pretty(&baseline::evaluate(&nixpkgs)?)? + "\n";
            if let Some(path) = output {
                #[cfg(unix)]
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                options.open(path)?.write_all(data.as_bytes())?;
            } else {
                print!("{data}");
            }
        }
        Command::Suggest {
            input,
            nixpkgs,
            output,
        } => {
            let inventory = read_inventory(&input)?;
            let report = suggest::evaluate(&inventory, &nixpkgs).map_err(std::io::Error::other)?;
            let found = report
                .suggestions
                .iter()
                .filter(|s| s.attribute_exists)
                .count();
            let data = serde_json::to_string_pretty(&report)? + "\n";
            if let Some(path) = output {
                #[cfg(unix)]
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                options.open(&path)?.write_all(data.as_bytes())?;
                eprintln!(
                    "Checked {} names ({} exact-name attributes exist; {} names require review) and wrote {}",
                    report.suggestions.len(),
                    found,
                    report.suggestions.len(),
                    path.display()
                );
            } else {
                print!("{data}");
            }
        }
        Command::Report {
            input,
            baseline: baseline_path,
            reviewed_mappings,
        } => {
            let inventory = read_inventory(&input)?;
            let reviewed = match reviewed_mappings {
                Some(path) => mapping::read_reviewed(&path).map_err(std::io::Error::other)?,
                None => mapping::ReviewedMappings::empty(),
            };
            let decisions = mapping::reviewed_decisions(&inventory, &reviewed, None)
                .map_err(std::io::Error::other)?;
            print!("{}", output::report(&inventory, &decisions));
            if let Some(path) = baseline_path {
                let mut bytes = Vec::new();
                File::open(path)?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 16 * 1024 * 1024 {
                    return Err("baseline exceeds 16 MiB".into());
                }
                let baseline: baseline::Baseline = serde_json::from_slice(&bytes)?;
                baseline::validate(&baseline).map_err(std::io::Error::other)?;
                let names = baseline::matches(&inventory, &baseline);
                println!("\n## Possible NixOS baseline overlaps\n\nMinimal NixOS evaluation: {} ({}, {}).", baseline.nixpkgs_version, baseline.system, baseline.nixpkgs_path);
                println!("{} explicit source package names match baseline Nixpkgs package names. Name matches do not prove equivalence, installation on your target, or that you personally installed them.\n", names.len());
                for name in names {
                    println!("- `{name}`");
                }
            }
        }
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        if e.downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
        {
            return;
        }
        eprintln!("nixify: {e}");
        std::process::exit(1);
    }
}
