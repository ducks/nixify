# Disposable, network-isolated test VM for a trusted generated draft.
# Evaluates the project and builds a VM; it never activates the host system.
{ nixpkgsPath, projectPath, system ? builtins.currentSystem }:
let
  nixpkgs = builtins.toPath nixpkgsPath;
  project = builtins.toPath projectPath;
in
import (nixpkgs + "/nixos/lib/eval-config.nix") {
  inherit system;
  modules = [
    (project + "/configuration.nix")
    (nixpkgs + "/nixos/modules/virtualisation/qemu-vm.nix")
    ({ lib, ... }: {
      # These VM-only settings are not written into the migration draft.
      system.stateVersion = lib.mkDefault "24.11";
      networking.hostName = lib.mkForce "nixify-test";
      services.getty.autologinUser = lib.mkForce "root";
      virtualisation = {
        memorySize = 2048;
        cores = 2;
        diskSize = 8192;
        graphics = false;
        useNixStoreImage = true;
        sharedDirectories = lib.mkForce {};
        qemu.networkingOptions = lib.mkForce [ "-nic" "none" ];
      };
    })
  ];
}
