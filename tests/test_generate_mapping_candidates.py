import gzip
import importlib.util
import json
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'generate_mapping_candidates.py'
spec = importlib.util.spec_from_file_location('generator', SCRIPT)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class BulkMappingTests(unittest.TestCase):
    def test_aur_ranked_with_unmatched_retained(self):
        data = [{'Name': 'first', 'Popularity': 9.0}, {'Name': 'second', 'Popularity': 4.0}, {'Name': 'last', 'Popularity': 1.0}]
        result = generator.build(generator.parse_aur(json.dumps(data)), {'legacyPackages.x86_64-linux.first': {'version': '1'}}, 'aur', 'a' * 40, 's', 'n', limit=2)
        self.assertEqual([x['source']['name'] for x in result['candidates']], ['first', 'second'])
        self.assertEqual(result['candidates'][1]['status'], 'no_exact_name_match')
        self.assertEqual(result['ranking_method'], 'aur_popularity_score')

    def test_debian_and_ubuntu_need_explicit_rankings(self):
        records = list(generator.parse_deb(b'Package: second\nVersion: 2\nDescription: Hello\n continuation\n\nPackage: first\nVersion: 1\n'))
        for registry in ['ubuntu', 'debian']:
            with self.assertRaises(ValueError):
                generator.build(records, {}, registry, 'a' * 40, 's', 'n')
            result = generator.build(records, {}, registry, 'a' * 40, 's', 'n', ['first', 'second'], 1)
            self.assertEqual(result['candidates'][0]['source']['name'], 'first')

    def test_malformed_index_and_duplicate_rankings_rejected(self):
        with self.assertRaises(ValueError):
            list(generator.parse_deb(b'Version: 1\n'))
        with self.assertRaises(ValueError):
            generator.build([], {}, 'ubuntu', 'a' * 40, 's', 'n', ['x', 'x'])
        with self.assertRaises(ValueError):
            generator.build(list(generator.parse_aur('[{"Name":"x"}]')), {}, 'aur', 'a' * 40, 's', 'n')


if __name__ == '__main__':
    unittest.main()
