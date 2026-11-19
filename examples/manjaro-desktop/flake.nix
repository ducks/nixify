# Migration draft: review every generated module before use.
{
  description = "nixify migration draft (not installable)";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/eeeb90a1dd3c9bea3afdbc76fd34d0fb2a727c7a";
  outputs = { nixpkgs, ... }: {
    nixosConfigurations.nixify-draft = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [ ./configuration.nix ];
    };
  };
}
