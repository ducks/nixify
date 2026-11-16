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

Generated `packages.nix` includes explicitly installed packages with curated mappings and inferred matches whose source and Nixpkgs catalog entries share a name and upstream homepage. The report labels inferred entries and their limitations, including when a foreign package happens to match AUR. Same-project packages may still differ, and foreign status does not prove AUR provenance. Dependencies are not promoted to direct packages; weak and unmapped matches remain in the report. Target revision, package features, platform support and behavior still need checking. Package mappings do not configure login shells, services, graphical sessions or application data. Output directories and inventory files must not exist already: existing work is never overwritten.

Some mapped packages may be classified as unfree by the selected Nixpkgs
revision. Attribute-existence and partial module checks do not guarantee a
system build will evaluate successfully. Review package licenses and configure
your own Nixpkgs policy where needed; nixify does not enable unfree packages.

`services.nix` proposes a small allowlist of NixOS module enablements only for persistently enabled system services. It never translates user services or treats a running service as evidence of persistent enablement. Review each service's configuration, data, credentials, security settings, and target options; enablement alone does not preserve behavior. The allowlist includes Bluetooth, printing, Avahi, libvirt, TLP, Ollama and Mullvad. On the checked Nixpkgs revision, Mullvad changes reverse-path firewall checking to loose; review target-specific effects before use. Service names and status are included in the inventory and may reveal sensitive information.


The generated flake pins the Nixpkgs commit used by nixify's own flake lock by default. To use a different full 40-character commit, pass `--nixpkgs-rev <commit>` to `generate`. No `flake.lock` is created until inputs are explicitly resolved. Supply reviewed hardware configuration, bootloader, filesystems, users, networking and an appropriate `system.stateVersion` before any installation. The `nixify validate` command requires Nix and checks syntax. Optionally use `nixify validate ./nixos-migration --nixpkgs /path/to/nixpkgs` to check the generated package attributes against a local checkout. The attribute check compares the generated flake, packages module, inventory and saved package decisions; it reports if the checkout's `.git-revision` differs from the target pin or is unavailable. A matching file is self-reported metadata, not authenticated checkout content. The check does not establish package equivalence, build, boot or activate a system.

For trusted drafts and Nixpkgs checkouts, `nixify validate ./nixos-migration --nixpkgs /path/to/nixpkgs --evaluate-modules` also evaluates the NixOS module graph, system package names and assertions with temporary placeholder root filesystem, bootloader and state-version settings. The check does not write those placeholders into the draft. It is a partial evaluation against the supplied checkout, which may differ from the pinned target; it does not build or prove hardware, service or boot compatibility. Nix evaluation can read files or fetch inputs named by the supplied code, so only evaluate trusted drafts and checkouts. A syntax pass is not a migration success.

`desktop.nix` contains commented suggestions for recognized desktop packages,
session files and process session hints. No desktop or display manager is
selected automatically. To choose one explicitly, add `--desktop hyprland` or
`--desktop xfce` to `nixify generate`. This enables the selected NixOS module
in the draft; it does not migrate desktop configuration, configure a login
manager, or prove a graphical session will work. For example:

```sh
nixify generate --input inventory.json --output ./nixos-migration --desktop hyprland
```

Installed packages and session hints do not establish the system default.
The report recognizes enabled Lemurs, but does not generate its configuration.
Confirm desktop and display-manager options for your target NixOS revision.

Hardware discovery uses `/proc/cpuinfo`, EFI interface presence, `lspci`, and
`lsblk` for planning evidence. Absence of the EFI interface is reported as
unknown, not BIOS; loop devices are omitted. No partition, filesystem or boot
configuration is generated. USB devices, fstab, Wi-Fi settings and detailed hardware
configuration are not scanned yet. Review the inventory before sharing it. Network inventory records interface-type counts, resolver categories and installed firewall package names; it does not retain interface names, addresses, DNS search domains, hostname or credentials. These summaries can still reveal aspects of your network or security posture.

## Optional NixOS hardware suggestions

If `nixos-generate-config` from `nixos-install-tools` is available on `PATH`,
nixify can capture its hardware-only output without asking it to write into
`/etc/nixos`:

```sh
nixify hardware-config --output hardware-configuration.nix
```

This optional command invokes `nixos-generate-config --show-hardware-config
--no-filesystems`; normal scanning and generation still work without Nix. The
new file is owner-only and never overwrites an existing file. Review it before
copying or importing it into the generated draft; nixify does not import it
automatically. Hardware suggestions may contain machine-identifying information.

`--include-filesystems` omits `--no-filesystems` and may include identifiers and
mounts for the **currently running source system**. Those are not necessarily
the intended NixOS target. This does not detect a future partition layout,
install NixOS, or establish that the generated system will boot.

## Configuration locations

The report identifies selected configuration locations for installed packages, including SSH, Docker, Lemurs, Hyprland, Waybar, Kitty, Neovim and Zsh. It checks metadata only and does not copy or interpret file contents. User locations refer to the scanning account’s `$HOME`; alternate locations and other users are not scanned. A symlink is reported without following it, and a missing file does not prove that the application has no configuration. Existing inventories without this field remain usable.

## Optional configuration inspection

File-content inspection is opt-in. Choose a profile and a specific regular file;
`inspect-config` never modifies the source, rejects final symlinks and files over
256 KiB, and creates a new owner-only JSON file. No raw file paths or contents are
stored. The profile determines which typed settings can be retained; all other
items are counted but omitted. The initial profiles are `hyprland` and
`docker-daemon`.

```sh
nixify inspect-config --profile hyprland --file ~/.config/hypr/hyprland.conf --output hypr-inspection.json
nixify generate --input inventory.json --output ./nixos-migration --desktop hyprland --config-inspection hypr-inspection.json
```

The Hyprland profile recognizes only `general.layout` (`dwindle` or `master`)
and `input.natural_scroll` booleans. It reports these for review; it does not
write user configuration or migrate keybindings, environment entries, commands,
includes or other settings. Includes and ambiguous blocks make the findings
incomplete and suppress values because they might be overridden.

The Docker profile recognizes only the JSON boolean `live-restore`, rejects
duplicate keys, and omits other settings. When an inspection is supplied to
`generate`, nixify writes the corresponding Docker option only if the inventory
shows the system Docker service persistently enabled. This does not migrate
Docker data or other daemon settings. Inspect and review the original files and
output; a successful inspection is not configuration equivalence. Add profiles
and typed translation rules for additional formats rather than accepting
arbitrary source keys as Nix expressions.

## Review more package mappings

Curated Manjaro mappings live in [mappings/manjaro.json](mappings/manjaro.json). An optional helper can collect public AUR and Arch package metadata for mapping review; see [package sources and contributions](docs/package-sources.md). This lookup is explicitly online and sends selected package names to the chosen registry.

An optional lookup checks unresolved explicit package names against a trusted local Nixpkgs checkout:

```sh
nixify suggest --input inventory.json --nixpkgs /path/to/nixpkgs --output suggestions.json
```

An exact-name attribute is only a **candidate**: its existence does not establish that it is installable, related, or equivalent. The lookup never changes the generated configuration and includes foreign packages without assuming they came from AUR. It assumes the source architecture for the target and reports the checkout's self-declared revision. Only evaluate trusted Nixpkgs code.

After checking individual packages yourself, create a review file. This example is illustrative; `nixpkgs_revision` may be omitted or set to a full commit hash:

```json
{
  "schema_version": 1,
  "nixpkgs_revision": null,
  "mappings": [
    {"source_package": "git", "nixpkgs_attribute": "git"}
  ]
}
```

Pass it explicitly to `nixify report --input inventory.json --reviewed-mappings review.json` or `nixify generate --input inventory.json --output ./nixos-migration --reviewed-mappings review.json`. Each source must be explicitly installed; foreign packages can also be overridden by explicit review. Attribute values are dot-separated Nixpkgs paths such as `xfce.exo`, not arbitrary Nix expressions; each path component is validated and quoted. If the review declares a revision, it must match the generated flake pin. The report labels these as user-reviewed, not automatically verified; use `validate --nixpkgs` to check attribute existence. Review files and suggestions disclose installed software and should be protected like inventories.

## Optional NixOS Search catalog

The `catalog-suggest` command reads the decompressed `packages.json` channel
catalog also used by NixOS Search's `flake-info`. It searches unresolved,
explicitly installed packages locally, including nested attribute names and
possible `-bin`/`-git` variants for foreign packages, plus selected distro
naming conventions such as `grep`/`gnugrep` and `xorg-xinit`/`xinit`. It does not contact a
server, select mappings, or modify the draft. Results are candidates only: a
name match does not establish equivalence, provenance, buildability or target
availability. Catalog reports disclose installed software.

For example, obtain a channel catalog and its revision from the **same resolved
release** (the channel itself moves). This requires `curl` and `brotli`, and
the decompressed JSON can occupy hundreds of megabytes:

```sh
curl -fsSL -o packages.json.br -w '%{url_effective}' \
  https://channels.nixos.org/nixos-26.05/packages.json.br > catalog-url.txt
catalog_url=$(cat catalog-url.txt)
curl -fsSL "${catalog_url%/*}/git-revision" -o catalog-revision.txt
brotli --decompress --keep packages.json.br
nixify catalog-suggest --input inventory.json --catalog packages.json \
  --revision "$(cat catalog-revision.txt)" --output catalog-matches.json
```

This example uses the NixOS 26.05 stable channel. The catalog is streamed and
limited to 1 GiB uncompressed. The revision is user-supplied metadata and is
not authenticated against the catalog; compare it with your intended target pin.
Nixify currently defaults its generated flake to the revision in its own
`flake.lock`, which tracks nixos-unstable. To use this catalog revision as the
target instead, pass `--nixpkgs-rev "$(cat catalog-revision.txt)"` to
`nixify generate`; the generated flake still needs to be resolved and validated. Downloading the public catalog does not send the
inventory to NixOS, and matching happens locally. An existing `packages.json`
from a fixed release can be reused offline. The report does not establish that
a match is available on the selected architecture. See the [NixOS Search
flake-info documentation](https://github.com/NixOS/nixos-search/tree/main/flake-info#nixpkgs).

## Optional NixOS baseline

With Nix installed, evaluate a local Nixpkgs checkout (this does not build or activate a system), then compare names:

```sh
nixify baseline --nixpkgs /path/to/nixpkgs --output baseline.json
nixify report --input inventory.json --baseline baseline.json
```

For a channel-based installation, the local path may be `/nix/var/nix/profiles/per-user/root/channels/nixpkgs`; use the checkout for your intended Nixpkgs revision when possible. The evaluation uses a minimal NixOS configuration and records the checkout path, Nixpkgs version, and platform. It compares explicit pacman names with NixOS `environment.systemPackages` package names. Exact name matches are **hints**, not proof of equivalence or a guarantee of what your eventual NixOS configuration will contain. The baseline does not automatically remove anything from generated output. Module options, NixOS versions, hardware, package variants and installed desktop all affect the actual system.

## Privacy

Inventories disclose installed package names, versions, origin flags, architecture and distribution. Treat them as sensitive. The scanner checks a small fixed set of system and scanning-user configuration paths for file presence; it does not read their contents, follow discovered symlinks, or retain the home directory path. It does not read private keys, credentials, Wi-Fi settings, or arbitrary environment variables. It reads only `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE` as optional session hints, retaining recognized values only; it checks known session-file paths without reading their contents. Hardware inventory includes CPU model, PCI display/network vendor and model, block-device types, sizes and filesystem types; these can identify your machine. It excludes device names, serials, UUIDs and non-system mount paths. Inventory and draft files are created with owner-only permissions on Unix. Review before sharing. A requested Nixpkgs baseline runs `nix-instantiate` on the specified checkout, so only evaluate a source you trust.

## Development

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
nix develop
nix build
nix flake check
```

Ordinary tests use synthetic pacman fixtures and need neither Nix nor Manjaro. See [architecture](docs/architecture.md), [limitations](docs/limitations.md), and the [isolated VM smoke test](docs/vm-testing.md).
