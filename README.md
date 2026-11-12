# nixify

`nixify` is a read-only Linux-to-NixOS migration assistant. The first milestone inventories Manjaro packages, systemd services, desktop signals and limited hardware and networking evidence, then produces conservative modules with a review report. It does not install NixOS, modify source configuration, execute service scripts, or partition disks.

This is **an early migration draft, not a bootable system**. Hardware discovery is partial and full NixOS system evaluation is not implemented yet. Package-attribute checks are available against an explicitly supplied local checkout. The generated flake is intentionally incomplete. Desktop discovery is limited to a small set of recognizable signals. Curated mappings must be verified against your selected Nixpkgs revision.

## Use

A compiled Rust binary runs on Manjaro without Nix. From source, install Rust and build with `cargo build --release`.

```sh
nixify scan --output inventory.json
nixify report --input inventory.json
nixify generate --input inventory.json --output ./nixos-migration
nixify validate ./nixos-migration
```

`scan` reads `/etc/os-release`, calls `pacman -Q`, `-Qqe`, and `-Qqm`, and queries system and user systemd scopes for unit-file and running states. It does not alter the package database or systemd configuration. If a systemd scope is inaccessible, its state is explicitly unknown. It collects package names, versions and install reasons. “Explicit” is a pacman database flag, **not a count of packages you personally installed**: an installer or setup tool may have marked packages explicit. “Foreign” means absent from configured sync databases, not necessarily AUR.

Generated `packages.nix` includes only explicitly installed, non-foreign packages with a curated mapping. Dependencies are not promoted to direct packages; foreign and unmapped packages remain in the report for review. The mapping database currently contains just git, Firefox, Neovim, ripgrep, curl and htop; no claim is made that they exist in a particular target revision until separately validated. Output directories and inventory files must not exist already: existing work is never overwritten.

`services.nix` proposes a small allowlist of NixOS module enablements only for persistently enabled system services. It never translates user services or treats a running service as evidence of persistent enablement. Review each service's configuration, data, credentials, security settings, and target options; enablement alone does not preserve behavior. Service names and status are included in the inventory and may reveal sensitive information.


The generated flake pins the Nixpkgs commit used by nixify's own flake lock by default. To use a different full 40-character commit, pass `--nixpkgs-rev <commit>` to `generate`. No `flake.lock` is created until inputs are explicitly resolved. Supply reviewed hardware configuration, bootloader, filesystems, users, networking and an appropriate `system.stateVersion` before any installation. The `nixify validate` command requires Nix and checks syntax. Optionally use `nixify validate ./nixos-migration --nixpkgs /path/to/nixpkgs` to check the generated package attributes against a local checkout. The attribute check requires the generated flake and packages module to remain unedited; it reports if the checkout's `.git-revision` differs from the target pin or is unavailable. A matching file is self-reported metadata, not authenticated checkout content. The check does not establish package equivalence, build, boot or activate a system.

For trusted drafts and Nixpkgs checkouts, `nixify validate ./nixos-migration --nixpkgs /path/to/nixpkgs --evaluate-modules` also evaluates the NixOS module graph, system package names and assertions with temporary placeholder root filesystem, bootloader and state-version settings. The check does not write those placeholders into the draft. It is a partial evaluation against the supplied checkout, which may differ from the pinned target; it does not build or prove hardware, service or boot compatibility. Nix evaluation can read files or fetch inputs named by the supplied code, so only evaluate trusted drafts and checkouts. A syntax pass is not a migration success.

`desktop.nix` contains commented suggestions for recognized desktop packages,
session files and process session hints. No desktop or display manager is
selected automatically. Installed packages and available sessions do not prove
which desktop is configured by default; confirm the options for your target
NixOS version before enabling one.

Hardware discovery uses `/proc/cpuinfo`, EFI interface presence, `lspci`, and
`lsblk` for planning evidence. Absence of the EFI interface is reported as
unknown, not BIOS; loop devices are omitted. No partition, filesystem or boot
configuration is generated. USB devices, fstab, Wi-Fi settings and detailed hardware
configuration are not scanned yet. Review the inventory before sharing it. Network inventory records interface-type counts, resolver categories and installed firewall package names; it does not retain interface names, addresses, DNS search domains, hostname or credentials. These summaries can still reveal aspects of your network or security posture.

## Optional NixOS baseline

With Nix installed, evaluate a local Nixpkgs checkout (this does not build or activate a system), then compare names:

```sh
nixify baseline --nixpkgs /path/to/nixpkgs --output baseline.json
nixify report --input inventory.json --baseline baseline.json
```

For a channel-based installation, the local path may be `/nix/var/nix/profiles/per-user/root/channels/nixpkgs`; use the checkout for your intended Nixpkgs revision when possible. The evaluation uses a minimal NixOS configuration and records the checkout path, Nixpkgs version, and platform. It compares explicit pacman names with NixOS `environment.systemPackages` package names. Exact name matches are **hints**, not proof of equivalence or a guarantee of what your eventual NixOS configuration will contain. The baseline does not automatically remove anything from generated output. Module options, NixOS versions, hardware, package variants and installed desktop all affect the actual system.

## Privacy

Inventories disclose installed package names, versions, origin flags, architecture and distribution. Treat them as sensitive. The scanner does not read private keys, credentials, home directories, Wi-Fi settings, arbitrary environment variables or service configuration. It reads only `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE` as optional session hints, retaining recognized values only; it checks known session-file paths without reading their contents. Hardware inventory includes CPU model, PCI display/network vendor and model, block-device types, sizes and filesystem types; these can identify your machine. It excludes device names, serials, UUIDs and non-system mount paths. Inventory and draft files are created with owner-only permissions on Unix. Review before sharing. A requested Nixpkgs baseline runs `nix-instantiate` on the specified checkout, so only evaluate a source you trust.

## Development

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
nix develop
nix build
nix flake check
```

Ordinary tests use synthetic pacman fixtures and need neither Nix nor Manjaro. See [architecture](docs/architecture.md) and [limitations](docs/limitations.md).
