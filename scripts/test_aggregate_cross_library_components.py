import unittest

from aggregate_cross_library_components import PAIRS, aggregate


class AggregateCrossLibraryComponentsTest(unittest.TestCase):
    def test_requires_ten_captures(self):
        with self.assertRaises(ValueError):
            aggregate([])

    def test_mapping_has_three_common_generation_cases(self):
        self.assertEqual(len(PAIRS), 3)
        self.assertTrue(all(pair[1][1] == "rsshogi_generate_move32" for pair in PAIRS.values()))


if __name__ == "__main__":
    unittest.main()
