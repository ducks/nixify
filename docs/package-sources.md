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

## Maintainer bulk candidate generation

`scripts/generate_mapping_candidates.py` creates a distribution-wide review queue without users submitting inventories. Download a snapshot of the [AUR metadata archive](https://wiki.archlinux.org/title/Aurweb_RPC_interface#AUR_metadata_archives), or a Debian/Ubuntu `Packages` index, and independently obtain `nix search --json` output from a pinned Nixpkgs revision. Supply the full revision explicitly. This tool reads local files only and records input hashes; it does not authenticate those files or the revision.

```sh
python3 scripts/generate_mapping_candidates.py \
  --registry aur --source packages-meta-v1.json.gz \
  --nix-search nix-search.json --nixpkgs-revision FULL_40_CHARACTER_COMMIT \
  --output aur-candidates.json
```

AUR defaults to the 1,000 packages with the highest AUR popularity score. This is not a download count. For Debian and Ubuntu, provide `--ranking ranked-packages.txt`, one package name per line in descending priority order. Debian's opt-in [popularity contest](https://popcon.debian.org/) is a possible source of installed-package counts; it is not a complete or representative download count. No Ubuntu ranking source is supplied or inferred. Record the source and date of any ranking when publishing results. Use `--limit` to adjust the number of unique packages considered.

Unmatched packages remain in the review queue, and exact-name matches are unreviewed. This script does not populate the runtime mapping database; source/target equivalence and the selected revision still need verification before a mapping is contributed. Users can recommend missing packages through project issues or pull requests. Debian and Ubuntu indexes are supported by the maintainer script, but nixify does not yet scan those distributions.

## Bundled candidate catalogs

The repository includes generated Manjaro repository and AUR candidate catalogs. They were produced from the local Manjaro sync databases (`core`, `extra`, `multilib`) and AUR metadata snapshot dated 2026-11-13, using an x86_64 Nixpkgs checkout declaring revision `eeeb90a1dd3c9bea3afdbc76fd34d0fb2a727c7a`. The checkout revision is self-reported. Each catalog records source archive hashes and individual upstream metadata. Manjaro candidates require an exact attribute name, a derivation, and matching normalized homepage. AUR candidates cover exact-name derivations in the 1,000 highest AUR popularity scores and indicate whether the homepages also match. AUR popularity is not a download count. Matching homepages can identify related but differently packaged components, and foreign status does not establish AUR provenance.

`nixify report` shows catalog candidates for explicit, otherwise unmapped packages; they are suggestions, not included in `packages.nix`. Use the reviewed mappings mechanism described in the README after checking the candidates. Catalog entries are independent of the user's inventory and need updating as repositories and Nixpkgs change. They are not verified against other Nixpkgs revisions or architectures.

The `build_aur_mappings.py` and `build_manjaro_candidates.py` maintainer tools accept a downloaded snapshot plus local Nixpkgs metadata evaluated with `evaluate_aur_candidates.nix`; see their `--help` for inputs. They do not fetch software or execute source package build scripts.
