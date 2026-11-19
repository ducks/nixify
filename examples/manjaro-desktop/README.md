# Example: Manjaro desktop migration draft

This is the complete set of Nix configuration files produced by nixify for a
real Manjaro desktop scan in November 2026. Hyprland was explicitly selected,
all application-review defaults were kept, and target package overlap was
evaluated against the Nixpkgs revision pinned in `flake.nix`. The 177 direct
package entries are an uncurated migration proposal, not a recommended package
set or a record of which applications the user deliberately installed.

The five `.nix` files are copied from the generated draft without changing the
configuration:

- `flake.nix`: the selected Nixpkgs revision and system entry point.
- `configuration.nix`: imports the generated modules and notes what is missing.
- `packages.nix`: proposed direct package entries.
- `services.nix`: proposed service enablements.
- `desktop.nix`: the selected Hyprland module and an XFCE suggestion.

**This is not an installable NixOS configuration.** It has no reviewed
hardware, filesystem or bootloader configuration, users, `system.stateVersion`,
or `flake.lock`. It has not been tailored to another machine. Some packages may
require an explicit unfree policy, and service settings and data are not
migrated. Review the choices and the Nixpkgs revision rather than copying them
blindly. An earlier draft was smoke-tested in a disposable VM; this exact
snapshot was evaluated with placeholder hardware but not boot-tested.

The original inventory, package decisions, selection file, target-coverage
evidence and migration report are omitted because they contain machine-specific
scan information. Consequently this example is for inspecting the generated Nix
files; it is not a complete `nixify validate` input. A fresh `nixify generate`
run creates the accompanying inventory and report for your own machine.

To check the syntax without building or activating anything, from this folder:

```sh
for file in *.nix; do nix-instantiate --parse "$file" >/dev/null || break; done
```
