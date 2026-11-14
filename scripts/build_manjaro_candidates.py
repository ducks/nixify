#!/usr/bin/env python3
"""Build reviewable Manjaro-to-Nixpkgs candidates from pacman sync databases."""
import argparse
import hashlib
import json
import re
import tarfile
from pathlib import Path
from build_aur_mappings import homepage


def read_databases(paths):
    records = {}
    for path in paths:
        with tarfile.open(path, 'r:*') as archive:
            for member in archive:
                if not member.isfile() or not member.name.endswith('/desc') or member.size > 1024 * 1024:
                    continue
                fields = {}
                key = None
                for line in archive.extractfile(member).read().decode('utf-8').splitlines():
                    if line.startswith('%') and line.endswith('%'):
                        key = line[1:-1]
                        fields[key] = []
                    elif line and key:
                        fields[key].append(line)
                if 'NAME' in fields:
                    name = fields['NAME'][0]
                    if name in records:
                        raise ValueError(f'duplicate package {name} across databases')
                    records[name] = {'source_package': name, 'source_homepage': fields.get('URL', [''])[0], 'repository': path.stem}
    return records


def build(records, metadata, revision, hashes):
    entries = []
    for name, record in sorted(records.items()):
        target = metadata.get(name)
        if not re.fullmatch(r'[A-Za-z0-9_+\-]+', name) or not isinstance(target, dict) or target.get('isDerivation') is not True:
            continue
        source_url = homepage(record['source_homepage'])
        target_url = homepage(target.get('homepage'))
        if source_url and source_url == target_url:
            entries.append({**record, 'nixpkgs_attribute': name, 'nixpkgs_homepage': target['homepage'], 'evidence': 'same_name_and_upstream_homepage'})
    return {'schema_version': 1, 'distribution': 'manjaro', 'nixpkgs_declared_revision': revision, 'repository_sha256': hashes, 'warning': 'Candidate mappings: matching name and upstream homepage do not prove equivalent behavior, licensing, target availability or configuration.', 'entries': entries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, action='append', required=True)
    parser.add_argument('--nix-metadata', type=Path, required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch('[0-9a-f]{40}', args.revision):
        parser.error('revision must be a 40-character hexadecimal commit')
    records = read_databases(args.repository)
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in args.repository}
    data = build(records, json.loads(args.nix_metadata.read_bytes()), args.revision, hashes)
    with args.output.open('x', encoding='utf-8') as out:
        json.dump(data, out, indent=2, ensure_ascii=False)
        out.write('\n')
    print(f"Wrote {len(data['entries'])} unreviewed repository candidates")


if __name__ == '__main__':
    main()
