#!/usr/bin/env python3
"""Create bundled AUR candidates from an AUR snapshot and evaluated Nixpkgs metadata."""
import argparse
import gzip
import hashlib
import json
import re
from pathlib import Path
from urllib.parse import urlsplit


def homepage(value):
    if not isinstance(value, str):
        return ''
    try:
        url = urlsplit(value.lower())
        if url.scheme not in ('http', 'https') or not url.hostname or url.username or url.password:
            return ''
        return (url.hostname.removeprefix('www.') + url.path.rstrip('/')).removesuffix('.git')
    except ValueError:
        return ''


def build(archive, nix_metadata, revision):
    source = json.loads(gzip.decompress(archive))
    source.sort(key=lambda item: (-item['Popularity'], item['Name']))
    entries = []
    for rank, item in enumerate(source[:1000], 1):
        name = item['Name']
        target = nix_metadata.get(name)
        if not re.fullmatch(r'[A-Za-z0-9_+\-]+', name) or not isinstance(target, dict) or target.get('isDerivation') is not True:
            continue
        source_homepage = homepage(item.get('URL'))
        target_homepage = homepage(target.get('homepage'))
        matches = bool(source_homepage and source_homepage == target_homepage)
        entries.append({'source_package': name, 'nixpkgs_attribute': name, 'aur_rank': rank, 'source_homepage': item.get('URL'), 'nixpkgs_homepage': target.get('homepage'), 'evidence': 'same_name_and_upstream_homepage' if matches else 'same_name_only'})
    entries.sort(key=lambda item: item['source_package'])
    return {'schema_version': 1, 'registry': 'aur', 'nixpkgs_declared_revision': revision, 'aur_archive_sha256': hashlib.sha256(archive).hexdigest(), 'selection': 'top 1000 by AUR popularity score', 'warning': 'Candidates only. Foreign does not prove AUR provenance; matching names or homepages do not prove equivalence, licensing, availability, or configuration compatibility.', 'entries': entries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--aur-archive', type=Path, required=True)
    parser.add_argument('--nix-metadata', type=Path, required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch('[0-9a-f]{40}', args.revision):
        parser.error('revision must be a 40-character hexadecimal commit')
    data = build(args.aur_archive.read_bytes(), json.loads(args.nix_metadata.read_bytes()), args.revision)
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(data, output, indent=2, ensure_ascii=False)
        output.write('\n')
    print(f"Wrote {len(data['entries'])} unreviewed mappings")


if __name__ == '__main__':
    main()
