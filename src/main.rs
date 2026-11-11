use clap::{Parser, Subcommand};
use nixify::{baseline, inventory::Inventory, manjaro, mapping, output};
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
#[derive(Subcommand)]
enum Command {
    /// Read-only Manjaro package inventory. No Nix installation is required.
    Scan {
        #[arg(long)]
        distro: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Generate a package module and migration report in a new directory.
    Generate {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Evaluate the default NixOS system packages from a local Nixpkgs checkout.
    Baseline {
        #[arg(long)]
        nixpkgs: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Explain curated mapping coverage and unresolved packages.
    Report {
        #[arg(long)]
        input: PathBuf,
        /// Optional name-only comparison against an evaluated NixOS baseline.
        #[arg(long)]
        baseline: Option<PathBuf>,
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
fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
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
        Command::Generate { input, output: dir } => {
            let inventory = read_inventory(&input)?;
            output::generate(&inventory, &dir)?;
            eprintln!("Wrote migration draft to {}", dir.display());
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
        Command::Report {
            input,
            baseline: baseline_path,
        } => {
            let inventory = read_inventory(&input)?;
            print!(
                "{}",
                output::report(&inventory, &mapping::decisions(&inventory))
            );
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
        eprintln!("nixify: {e}");
        std::process::exit(1);
    }
}
