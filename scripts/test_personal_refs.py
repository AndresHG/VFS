"""Tests for personal_refs.py: kinship phrases are flagged, ordinary text is not.

Run: python3 -m unittest discover -s scripts -p "test_*.py"
"""

import importlib.util
import os
import unittest

_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "personal_refs.py")
_SPEC = importlib.util.spec_from_file_location("personal_refs", _PATH)
refs = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(refs)


class FindHitsTest(unittest.TestCase):
    def test_flags_first_person_kinship_in_english_and_spanish(self):
        for text in (
            "Built for my mother's old laptop.",
            "Our kids keep photos here.",
            "Lo usa mi padre a diario.",
            "Fotos de mis abuelos",
            "Revisar con mamá",
            "Ask Dad first",
        ):
            with self.subTest(text=text):
                self.assertEqual(len(refs.find_hits(text)), 1)

    def test_ignores_generic_wording_and_lookalikes(self):
        for text in (
            "The target user searches by topic; they rarely type full names.",
            "Recetas de la abuela",
            "parent folder, child process, sibling crate",
            "Damage, mummy, momentum, dadaist",
            "Los resultados son mis favoritos",
        ):
            with self.subTest(text=text):
                self.assertEqual(refs.find_hits(text), [])

    def test_reports_line_numbers(self):
        self.assertEqual(refs.find_hits("ok\nmy wife\n")[0][0], 2)


if __name__ == "__main__":
    unittest.main()
