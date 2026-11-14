{ namesFile, nixpkgsPath }:
let
  pkgs = import (builtins.toPath nixpkgsPath) { system = "x86_64-linux"; config.allowUnfree = true; };
  names = builtins.fromJSON (builtins.readFile namesFile);
  inspect = name:
    let probe = builtins.tryEval (let
      p = builtins.getAttr name pkgs;
      homepage = p.meta.homepage or "";
      value = { isDerivation = pkgs.lib.isDerivation p; homepage = if builtins.isString homepage then homepage else ""; description = p.meta.description or ""; version = p.version or ""; };
    in builtins.deepSeq value value);
    in { inherit name; value = if builtins.hasAttr name pkgs && probe.success then probe.value else null; };
in builtins.listToAttrs (map inspect names)
