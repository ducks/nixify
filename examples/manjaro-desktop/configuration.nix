# Incomplete migration draft: not a bootable or installable configuration.
# Add reviewed hardware configuration, filesystems, bootloader, users, networking,
# and system.stateVersion appropriate for the NixOS installation before use.
{ ... }:
{
  imports = [ ./packages.nix ./services.nix ./desktop.nix ];
}
