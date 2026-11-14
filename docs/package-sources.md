# Package mapping sources

Nixify's curated [Manjaro mappings](../mappings/manjaro.json) are checked into the repository and are validated when the program starts. They are deliberately small. Existing entries record a Nixpkgs revision and a derivation-existence check; this does not establish package equivalence. The revision is the checkout's self-reported revision, not an authenticated identity. The existing entries need more source-package evidence and community review.

The optional helper gathers metadata from the [AUR RPC API](https://wiki.archlinux.org/title/Aurweb_RPC_interface) or the [Arch package search API](https://wiki.archlinux.org/title/Official_repositories_web_interface) to help review candidates. It uses Python 3's standard library. First create suggestions using a trusted local Nixpkgs checkout:

```sh
nixify suggest --input inventory.json --nixpkgs /path/to/nixpkgs --output suggestions.json
python3 scripts/enrich_candidates.py --input suggestions.json --registry aur --output aur-evidence.json --online
python3 scripts/enrich_candidates.py --input suggestions.json --registry arch --output arch-evidence.json --online
```

The helper sends selected package names and your IP address to the chosen public service. It queries AUR for foreign packages, and Arch for non-foreign packages. A foreign package is not necessarily from AUR; Arch's index is not authoritative for Manjaro repositories. An AUR match is not proof of installation provenance. Results can be stale, and a shared name does not establish equivalence or that the package is suitable for your target system. Exact-name Nixpkgs candidates indicate only whether the attribute exists, not whether it is a derivation or equivalent.

Lookups are limited to 30 names by default; use `--limit` (maximum 50) and `--offset` for another batch. The helper makes no requests without `--online`, does not fetch or run package build scripts, and does not edit or accept mappings. Failures stop the lookup without producing a report. It creates a new owner-only report and refuses to overwrite existing files. Reports reveal software choices; review and remove personal information before sharing them. For large-scale indexing, follow each registry's API and rate-limit guidance rather than repeatedly querying it.

## Contributing mappings

Check the original package and target Nixpkgs attribute against their upstream sources, including any differences in functionality, licensing and platform support. Check that the target attribute is a derivation on the relevant Nixpkgs revision. Propose a reviewed entry in `mappings/manjaro.json`, with the actual revision, and explain the source and equivalence evidence in the pull request. A registry name or description alone is not sufficient. Entries that need manual configuration should say so; ambiguous cases should remain manual review. Never include an unredacted personal inventory in a public contribution.

Run `cargo fmt --all -- --check`, `cargo test --locked`, `cargo clippy --all-targets --locked -- -D warnings`, and `python3 -m unittest discover -s tests -p 'test_enrich_candidates.py' -v`. Run `nix flake check` where Nix is available.
