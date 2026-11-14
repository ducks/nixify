import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'enrich_candidates.py'
spec = importlib.util.spec_from_file_location('enrich_candidates', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def suggestion(name, foreign=False, exists=True):
    return {'source_package': name, 'candidate_attribute': name, 'foreign': foreign, 'attribute_exists': exists}


class EnrichmentTests(unittest.TestCase):
    def test_selection_is_bounded_sorted_and_distinguishes_foreign(self):
        data = {'schema_version': 1, 'suggestions': [suggestion('z', True), suggestion('b'), suggestion('a', True)]}
        selected, total = module.select_suggestions(data, 'aur', 1, 1)
        self.assertEqual(total, 2)
        self.assertEqual(selected[0]['name'], 'z')
        self.assertEqual(module.select_suggestions(data, 'arch', 0, 30)[0][0]['name'], 'b')
        with self.assertRaises(ValueError):
            module.select_suggestions(data, 'aur', 0, 51)

    def test_untrusted_suggestions_are_rejected(self):
        for entries in ([suggestion('x'), suggestion('x')], [suggestion('../bad')], [dict(suggestion('ok'), candidate_attribute='evil')], [dict(suggestion('ok'), foreign=1)]):
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                module.select_suggestions({'schema_version': 1, 'suggestions': entries}, 'aur', 0, 30)

    def test_aur_evidence_and_missing_are_unreviewed(self):
        def fetch(url):
            self.assertEqual(urlsplit(url).hostname, 'aur.archlinux.org')
            self.assertEqual(parse_qs(urlsplit(url).query)['arg[]'], ['found', 'missing'])
            return {'version': 5, 'type': 'multiinfo', 'resultcount': 1, 'results': [{'Name': 'found', 'Version': '1', 'Description': 'description', 'URL': 'https://example.org/project?secret=x'}]}
        data = {'schema_version': 1, 'suggestions': [suggestion('missing', True, False), suggestion('found', True)]}
        report = module.enrich(data, 'aur', 0, 30, fetch)
        self.assertEqual([x['status'] for x in report['results']], ['registry_name_found', 'not_found_in_registry'])
        self.assertEqual(report['results'][0]['registry_records'][0]['upstream_homepage'], 'https://example.org/project')
        self.assertIn('not package equivalence', report['warning'])

    def test_malformed_and_unexpected_responses_fail(self):
        for response in ({'version': 5, 'type': 'error', 'results': []}, {'version': 5, 'type': 'multiinfo', 'results': [{'Name': 'other'}]}):
            with self.subTest(response=response), self.assertRaises(ValueError):
                module.aur_lookup(['pkg'], lambda _: response)
        with self.assertRaises(ValueError):
            module.arch_lookup(['pkg'], lambda _: {'next': 'next', 'results': []})
        with self.assertRaises(ValueError):
            module.arch_lookup(['pkg'], lambda _: {'results': [{'pkgname': 'other'}]})

    def test_arch_exact_name_and_multiple_architectures(self):
        def fetch(url):
            self.assertEqual(urlsplit(url).hostname, 'archlinux.org')
            self.assertEqual(parse_qs(urlsplit(url).query)['name'], ['pkg'])
            return {'next': None, 'results': [{'pkgname': 'pkg', 'pkgver': '1'}, {'pkgname': 'pkg', 'pkgver': '2'}]}
        self.assertEqual(len(module.arch_lookup(['pkg'], fetch)['pkg']), 2)

    def test_public_url_redaction(self):
        self.assertIsNone(module.public_homepage('https://user:password@example.org/'))
        self.assertIsNone(module.public_homepage('file:///etc/passwd'))
        self.assertEqual(module.public_homepage('https://example.org/a?token=x#foo'), 'https://example.org/a')

    def test_online_opt_in_required(self):
        result = subprocess.run([sys.executable, str(SCRIPT), '--input', '/does/not/exist', '--registry', 'aur', '--output', '/does/not/exist'], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('--online is required', result.stderr)


if __name__ == '__main__':
    unittest.main()
