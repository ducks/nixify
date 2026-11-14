#!/usr/bin/env python3
"""Enrich local nixify suggestions with opt-in public registry metadata.

This sends selected package names to AUR or Arch Linux. It never installs a
package, downloads build scripts, follows package homepages or writes mappings.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

MAX_INPUT = 4 * 1024 * 1024
MAX_RESPONSE = 2 * 1024 * 1024
PACKAGE = re.compile(r"[A-Za-z0-9@._+\-]{1,255}\Z")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(req.full_url, code, "redirect refused", headers, fp)


def public_text(value, limit=500):
    if not isinstance(value, str):
        return None
    value = " ".join(value.split())
    if any(ord(c) < 32 or ord(c) == 127 for c in value):
        return None
    return value[:limit] if value else None


def public_homepage(value):
    if not isinstance(value, str) or len(value) > 2048 or any(ord(c) < 32 or ord(c) == 127 for c in value):
        return None
    try:
        url = urllib.parse.urlsplit(value)
        if url.scheme not in ("http", "https") or not url.hostname or url.username or url.password:
            return None
        return urllib.parse.urlunsplit((url.scheme, url.netloc, url.path, "", ""))
    except ValueError:
        return None


def request_json(url, opener=None):
    opener = opener or urllib.request.build_opener(NoRedirect())
    request = urllib.request.Request(url, headers={"User-Agent": "nixify-candidate-evidence/0.1", "Accept": "application/json"})
    try:
        with opener.open(request, timeout=10) as response:
            data = response.read(MAX_RESPONSE + 1)
    except (OSError, urllib.error.URLError) as exc:
        raise ValueError(f"registry request failed: {type(exc).__name__}") from exc
    if len(data) > MAX_RESPONSE:
        raise ValueError("registry response exceeds 2 MiB")
    try:
        return json.loads(data)
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise ValueError("registry returned invalid JSON") from exc


def select_suggestions(data, registry, offset, limit):
    if not isinstance(data, dict) or data.get("schema_version") != 1 or not isinstance(data.get("suggestions"), list):
        raise ValueError("expected a nixify suggestions schema version 1")
    if offset < 0 or not 1 <= limit <= 50:
        raise ValueError("offset must be nonnegative and limit must be between 1 and 50")
    selected = []
    seen = set()
    for item in data["suggestions"]:
        if not isinstance(item, dict):
            raise ValueError("invalid suggestion entry")
        name, attribute = item.get("source_package"), item.get("candidate_attribute")
        if not isinstance(name, str) or not PACKAGE.fullmatch(name) or attribute != name or type(item.get("foreign")) is not bool or type(item.get("attribute_exists")) is not bool or name in seen:
            raise ValueError("invalid or duplicate package suggestion")
        seen.add(name)
        # AUR is queried only for foreign entries; Arch is supplementary metadata
        # for non-foreign entries, not evidence of Manjaro repository provenance.
        if item["foreign"] == (registry == "aur"):
            selected.append({"name": name, "nixpkgs_exact_name_exists": item["attribute_exists"], "foreign": item["foreign"]})
    selected.sort(key=lambda item: item["name"])
    return selected[offset:offset + limit], len(selected)


def aur_batches(names):
    batch = []
    for name in names:
        proposed = batch + [name]
        url = "https://aur.archlinux.org/rpc/v5/info?" + urllib.parse.urlencode([("arg[]", n) for n in proposed])
        if len(url) > 3500 or len(proposed) > 25:
            if not batch:
                raise ValueError("package name exceeds safe URL limit")
            yield batch
            batch = [name]
        else:
            batch = proposed
    if batch:
        yield batch


def aur_lookup(names, fetch=request_json):
    results = {}
    for batch in aur_batches(names):
        url = "https://aur.archlinux.org/rpc/v5/info?" + urllib.parse.urlencode([("arg[]", n) for n in batch])
        payload = fetch(url)
        if not isinstance(payload, dict) or payload.get("version") != 5 or payload.get("type") != "multiinfo" or not isinstance(payload.get("results"), list):
            raise ValueError("unexpected AUR response; no results were written")
        for item in payload["results"]:
            if not isinstance(item, dict) or item.get("Name") not in batch or item["Name"] in results:
                raise ValueError("AUR returned unexpected or duplicate package metadata")
            name = item["Name"]
            results[name] = [{"name": name, "version": public_text(item.get("Version")), "description": public_text(item.get("Description")), "upstream_homepage": public_homepage(item.get("URL")), "registry_page": "https://aur.archlinux.org/packages/" + urllib.parse.quote(name, safe="")}]
    return results


def arch_lookup(names, fetch=request_json):
    results = {}
    for index, name in enumerate(names):
        if index:
            time.sleep(0.15)
        url = "https://archlinux.org/packages/search/json/?" + urllib.parse.urlencode({"name": name})
        payload = fetch(url)
        if not isinstance(payload, dict) or not isinstance(payload.get("results"), list) or payload.get("next"):
            raise ValueError("unexpected or paginated Arch response; no results were written")
        records = []
        for item in payload["results"]:
            if not isinstance(item, dict) or item.get("pkgname") != name:
                raise ValueError("Arch returned unexpected package metadata")
            records.append({"name": name, "version": public_text(item.get("pkgver")), "description": public_text(item.get("pkgdesc")), "upstream_homepage": public_homepage(item.get("url")), "registry_page": "https://archlinux.org/packages/?" + urllib.parse.urlencode({"name": name})})
        if records:
            results[name] = records
    return results


def enrich(data, registry, offset, limit, fetch=request_json):
    selected, total = select_suggestions(data, registry, offset, limit)
    names = [item["name"] for item in selected]
    lookup = aur_lookup(names, fetch) if registry == "aur" else arch_lookup(names, fetch)
    for item in selected:
        item["registry_records"] = lookup.get(item["name"], [])
        item["status"] = "registry_name_found" if item["registry_records"] else "not_found_in_registry"
    return {
        "schema_version": 1,
        "registry": registry,
        "queried_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "nixpkgs_checkout": public_text(data.get("nixpkgs_checkout"), 2048),
        "nixpkgs_declared_revision": public_text(data.get("checkout_declared_revision")),
        "target_system": public_text(data.get("target_system")),
        "offset": offset,
        "eligible_count": total,
        "results": selected,
        "warning": "Registry records and Nixpkgs name matches are unreviewed evidence, not package equivalence or proof of installation provenance. Arch metadata is not authoritative for Manjaro repositories. No mappings were accepted or generated.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True, help="JSON produced by nixify suggest")
    parser.add_argument("--registry", choices=("aur", "arch"), required=True)
    parser.add_argument("--output", type=Path, required=True, help="New owner-only JSON output file")
    parser.add_argument("--offset", type=int, default=0)
    parser.add_argument("--limit", type=int, default=30)
    parser.add_argument("--online", action="store_true", help="Explicitly send selected package names to the chosen public registry")
    args = parser.parse_args()
    if not args.online:
        parser.error("--online is required: package names will be sent to the selected public registry")
    with args.input.open("rb") as file:
        content = file.read(MAX_INPUT + 1)
    if len(content) > MAX_INPUT:
        raise ValueError("suggestions input exceeds 4 MiB")
    data = json.loads(content)
    report = enrich(data, args.registry, args.offset, args.limit)
    encoded = (json.dumps(report, indent=2, ensure_ascii=False) + "\n").encode()
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as file:
        file.write(encoded)
    print(f"Wrote {len(report['results'])} unreviewed {args.registry} records to {args.output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"candidate enrichment: {error}", file=sys.stderr)
        sys.exit(1)
