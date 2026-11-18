# Isolated NixOS VM smoke test

This optional test builds a **disposable VM**, not an installer. It imports a trusted generated `configuration.nix` using a chosen local Nixpkgs checkout. The VM-only harness supplies `system.stateVersion`, uses an 8 GiB virtual disk, gives the serial console an automatic root login, disables its network interface, and configures no shared host directories. Its Nix store is copied into a separate VM image. Never use this harness as a production configuration or with untrusted generated Nix code: Nix evaluation can read or fetch inputs requested by that code.

The build writes to the local Nix store and may download packages and consume substantial disk space. The VM disk and store image live under a new private `.nixify-vm-*` directory in the repository; no physical disk is attached. QEMU and Nix must work on the host. KVM speeds up the test; the smoke runner requests software emulation when `/dev/kvm` is unavailable. Generated packages and services are only the mappings nixify currently knows about; desktop options are comments unless explicitly selected, and this is not a graphical-desktop or migration-equivalence test.

From a normal Manjaro terminal, in the nixify repository:

```sh
cargo build --release
nixpkgs_path=$(readlink -f /nix/var/nix/profiles/per-user/root/channels/nixpkgs)
nixpkgs_rev=$(cat "$nixpkgs_path/.git-revision")
vm_work=$(mktemp -d "$PWD/.nixify-vm-XXXXXXXX")
chmod 700 "$vm_work"
./target/release/nixify scan --output "$vm_work/inventory.json"
./target/release/nixify generate --input "$vm_work/inventory.json" \
  --output "$vm_work/draft" --nixpkgs-rev "$nixpkgs_rev"
nix-build tests/vm-test.nix -A config.system.build.vm --no-out-link \
  --argstr nixpkgsPath "$nixpkgs_path" --argstr projectPath "$vm_work/draft" \
  > "$vm_work/vm-path"
python3 tests/run-vm-smoke.py \
  "$(cat "$vm_work/vm-path")/bin/run-nixify-test-vm" "$vm_work"
```

The channel path and `.git-revision` above are examples; use a trusted checkout with a known revision if your host is configured differently. The harness imports the checkout directly, rather than resolving the generated flake input. Matching `.git-revision` is self-reported metadata, not content authentication.

The runner checks the generated VM script for the expected network and host-share settings; use it **only with the trusted VM produced by this harness**, not arbitrary VM scripts. It starts a fresh virtual disk, waits for the guest console, reports the presence of a few example commands and systemd state, then powers off the guest. Console output is saved to `runtime/console.log`, and both disk and store image remain inside `$vm_work`. Protect the inventory and logs as machine-sensitive data. You can remove this test directory yourself when you no longer need it. Since `--no-out-link` creates no GC root, Nix garbage collection may remove the built VM closure later.

Add `--expect-command Hyprland` (or repeat the option for other executables) to
fail the smoke test if a required command is unavailable. Command availability
is not evidence that the graphical session or application works correctly.
If the build fails on an unfree package, review the package and target license
policy before proceeding; the generated draft deliberately does not enable it.
The runner's 30-minute default timeout includes creating and populating the VM's
private store image, which can take a long time for a large closure. Override it
with `--timeout` (seconds) when appropriate.

For the console parser's offline tests, run:

```sh
python3 -m unittest discover -s tests -p 'test_vm_smoke.py' -v
```
