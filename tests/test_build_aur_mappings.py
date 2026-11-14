import gzip
import importlib.util
import json
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('aur', Path(__file__).resolve().parents[1] / 'scripts/build_aur_mappings.py')
aur = importlib.util.module_from_spec(spec)
spec.loader.exec_module(aur)


class AurMappingTests(unittest.TestCase):
    def test_records_distinguish_homepage_evidence_and_skip_missing(self):
        source = [{'Name': 'yes', 'Popularity': 3, 'URL': 'https://www.example.org/foo/'}, {'Name': 'weak', 'Popularity': 2, 'URL': 'https://one.example/'}, {'Name': 'missing', 'Popularity': 1}]
        metadata = {'yes': {'isDerivation': True, 'homepage': 'http://example.org/foo'}, 'weak': {'isDerivation': True, 'homepage': 'https://two.example/'}}
        result = aur.build(gzip.compress(json.dumps(source).encode()), metadata, 'a' * 40)
        self.assertEqual(len(result['entries']), 2)
        self.assertEqual([x['evidence'] for x in result['entries']], ['same_name_only', 'same_name_and_upstream_homepage'])
        self.assertEqual(result['entries'][1]['aur_rank'], 1)

    def test_no_private_credentials_in_homepage_match(self):
        self.assertEqual(aur.homepage('https://user:secret@example.org/foo'), '')


if __name__ == '__main__':
    unittest.main()
