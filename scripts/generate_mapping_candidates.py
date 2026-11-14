#!/usr/bin/env python3
"""Build a review queue from downloaded public package indexes and nix search JSON.

This is an offline maintainer tool. It never changes nixify's curated mappings.
"""
import argparse
import gzip
import hashlib
import json
import re
from pathlib import Path

NAME = re.compile(r'[A-Za-z0-9@._+\-]{1,255}\Z')


def read_bytes(path):
    data = path.read_bytes()
    return gzip.decompress(data) if data.startswith(b'\x1f\x8b') else data


def clean(value):
    return ' '.join(value.split())[:500] if isinstance(value, str) else None


def parse_aur(data):
    records = json.loads(data)
    if not isinstance(records, list):
        raise ValueError('AUR archive must be a JSON array')
    for record in records:
        if not isinstance(record, dict) or not isinstance(record.get('Name'), str):
            raise ValueError('invalid AUR record')
        yield {'name': record['Name'], 'version': clean(record.get('Version')), 'description': clean(record.get('Description')), 'homepage': clean(record.get('URL')), 'popularity': record.get('Popularity')}


def parse_deb(data):
    for paragraph in re.split(r'\n\s*\n', data.decode('utf-8')):
        fields = {}
        for line in paragraph.splitlines():
            if line.startswith((' ', '\t')):
                continue
            if ':' in line:
                key, value = line.split(':', 1)
                fields[key] = value.strip()
        if not fields:
            continue
        if 'Package' not in fields:
            raise ValueError('package index paragraph missing Package field')
        yield {'name': fields['Package'], 'version': clean(fields.get('Version')), 'description': clean(fields.get('Description')), 'homepage': clean(fields.get('Homepage'))}


def build(source, nixpkgs, registry, revision, source_hash, nix_hash, ranking=None, limit=1000):
    # nix search JSON keys typically include legacyPackages.<system>.
    targets = {}
    for key, metadata in nixpkgs.items():
        if not isinstance(metadata, dict):
            continue
        parts = key.split('.')
        if len(parts) != 3 or parts[0] != 'legacyPackages' or not NAME.fullmatch(parts[2]):
            continue
        targets.setdefault(parts[2], []).append({'attribute': key, 'version': clean(metadata.get('version')), 'description': clean(metadata.get('description'))})
    records = list(source)
    for record in records:
        if not NAME.fullmatch(record['name']):
            raise ValueError(f"invalid package name: {record['name']!r}")
    if ranking is None:
        if registry != 'aur':
            raise ValueError('Debian and Ubuntu require an explicit ranked package list')
        if any(not isinstance(r.get('popularity'), (int, float)) or isinstance(r.get('popularity'), bool) for r in records):
            raise ValueError('AUR records must include numeric popularity scores')
        records.sort(key=lambda r: (-r['popularity'], r['name']))
        ranking_method = 'aur_popularity_score'
    else:
        order = {name: rank for rank, name in enumerate(ranking)}
        if len(order) != len(ranking) or any(not NAME.fullmatch(name) for name in ranking):
            raise ValueError('ranking must contain unique valid package names')
        records = [record for record in records if record['name'] in order]
        records.sort(key=lambda r: (order[r['name']], r['version'] or '', r['description'] or ''))
        ranking_method = 'provided_ranked_list'
    selected_names = list(dict.fromkeys(record['name'] for record in records))[:limit]
    selected = set(selected_names)
    candidates = []
    for record in records:
        name = record['name']
        if name in selected:
            matches = sorted(targets.get(name, []), key=lambda x: x['attribute'])
            candidates.append({'source': record, 'nixpkgs_candidates': matches, 'status': 'unreviewed_exact_name_match' if matches else 'no_exact_name_match'})
    return {'schema_version': 1, 'registry': registry, 'nixpkgs_revision': revision, 'source_sha256': source_hash, 'nix_search_sha256': nix_hash, 'ranking_method': ranking_method, 'selected_package_count': len(selected_names), 'candidates': candidates, 'warning': 'Exact names are candidates only, not verified mappings, provenance, or equivalent behavior. Review source and target before use.'}



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True, help='Downloaded AUR metadata JSON or Debian/Ubuntu Packages index (optionally gzip)')
    parser.add_argument('--registry', choices=['aur', 'debian', 'ubuntu'], required=True)
    parser.add_argument('--nix-search', type=Path, required=True, help='JSON from nix search --json against a pinned Nixpkgs revision')
    parser.add_argument('--nixpkgs-revision', required=True, help='Full revision of the Nixpkgs snapshot used for nix search')
    parser.add_argument('--output', type=Path, required=True, help='New JSON review queue')
    parser.add_argument('--ranking', type=Path, help='One package name per line, ranked highest first; required for Debian and Ubuntu')
    parser.add_argument('--limit', type=int, default=1000, help='Maximum unique source packages (default: 1000)')
    args = parser.parse_args()
    if not 1 <= args.limit <= 100000:
        parser.error('--limit must be between 1 and 100000')
    if not re.fullmatch(r'[0-9a-f]{40}', args.nixpkgs_revision):
        parser.error('--nixpkgs-revision must be a full lowercase 40-character commit hash')
    raw_source = args.source.read_bytes()
    raw_nix = args.nix_search.read_bytes()
    source_data = read_bytes(args.source)
    records = parse_aur(source_data) if args.registry == 'aur' else parse_deb(source_data)
    nixpkgs = json.loads(raw_nix)
    if not isinstance(nixpkgs, dict):
        raise ValueError('nix search JSON must be an object')
    result = build(records, nixpkgs, args.registry, args.nixpkgs_revision, hashlib.sha256(raw_source).hexdigest(), hashlib.sha256(raw_nix).hexdigest(), args.ranking.read_text(encoding='utf-8').splitlines() if args.ranking else None, args.limit)
    result['ranking_sha256'] = hashlib.sha256(args.ranking.read_bytes()).hexdigest() if args.ranking else None
    with args.output.open('x', encoding='utf-8') as file:
        json.dump(result, file, indent=2, ensure_ascii=False)
        file.write('\n')
    print(f"Wrote {len(result['candidates'])} unreviewed candidates to {args.output}")


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, UnicodeError) as error:
        raise SystemExit(f'mapping candidates: {error}')
