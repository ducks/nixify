# Migration draft: enablement only; review service configuration and target options.
{ ... }:
{
  hardware.bluetooth.enable = true;
  networking.networkmanager.enable = true;
  services.avahi.enable = true;
  services.mullvad-vpn.enable = true;
  services.ollama.enable = true;
  services.printing.enable = true;
  services.tlp.enable = true;
  virtualisation.docker.enable = true;
  virtualisation.libvirtd.enable = true;
}
