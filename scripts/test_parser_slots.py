import unittest
from parser_slots import substitute, semantic_safe


class SlotTests(unittest.TestCase):
    def test_exact_round_trips(self):
        for text in [
            'copy "résumé 猫.pdf" from "Client Reports" to "Backup"',
            "rename King's report.pdf to \"O'Brien's final.pdf\"",
            "copy 'King's report.pdf' to Documents",
            'copy ~/Documents/猫.pdf to ~/Desktop',
            'list files larger than 2.5 GB and smaller than 3000 MB in Downloads',
            'extract pages 2 to 7 from report.pdf',
            'replace " " with "_" in names of files in Downloads',
            'replace "x" with "x" in names of files',
            'list PDFs in Downloads from this week',
            'resize image.png to 640 px',
            'list files over 2 GB and under 2048 MB',
        ]:
            with self.subTest(text=text):
                bundle = substitute(text)
                self.assertEqual(bundle.restore(bundle.encoded), text)

    def test_bad_slots_and_incompatible_units_are_refused(self):
        b = substitute('resize photo.png to 640 px')
        for rewrite in ['resize NAME1 to NUM2', 'resize NAME1 to NUM1 NUM1',
                        'resize NAME1 to 640 px', 'resize NAME 1 to NUM1', 'resize NAME1 to NUM1 MB',
                        'resize NAME1 to NUM1 and copy extra.pdf', 'resize NAME1 to NUM1 7',
                        'resize "NAME1" to NUM1']:
            with self.subTest(rewrite=rewrite), self.assertRaises(ValueError):
                b.restore(rewrite)
        self.assertEqual(b.restore('REFUSE'), 'REFUSE')
        with self.assertRaises(ValueError):
            substitute('copy NAME1 to Documents')

    def test_target_conversion_uses_existing_value_and_preserves_symbolic_dates(self):
        b = substitute('show files over 2.5 GB in Downloads from this week')
        encoded = b.encode_target('list files larger than 2560 MB in Downloads from this week')
        self.assertIn('this week', encoded)
        restored = b.restore(encoded)
        self.assertIn('2.5 GB', restored)
        self.assertNotIn('2560', restored)
        with self.assertRaises(ValueError):
            b.encode_target('list files larger than 2 MB in Downloads from this week')

    def test_replacement_direction(self):
        b = substitute('replace "old" with "new" in names of files')
        with self.assertRaises(ValueError):
            b.restore('replace NAME2 with NAME1 in names of files')
        self.assertEqual(b.restore('replace NAME1 with NAME2 in names of files'), b.original)

    def test_source_destination_and_slot_type_compatibility(self):
        b = substitute('move report.pdf to Documents')
        with self.assertRaises(ValueError):
            b.restore('move FOLDER1 to NAME1')
        self.assertEqual(b.restore('move NAME1 into FOLDER1'), b.original.replace(' to ', ' into '))
        pages = substitute('extract pages 2 to 7 from report.pdf')
        with self.assertRaises(ValueError):
            pages.restore('extract pages NUM2 to NUM1 from NAME1')

    def test_actions_filters_units_and_unsupported_requests(self):
        for source, rewrite in [
            ('list files over 2 GB', 'delete files larger than 2 GB'),
            ('move files over 2 GB to Archive', 'move files smaller than 2 GB to Archive'),
            ('move files over 2 GB to Archive', 'move files larger than 2 MB to Archive'),
            ('list PDFs excluding receipts', 'list PDFs'),
            ('move PDFs from this week to Archive', 'move PDFs since October 26 to Archive'),
            ('move report.pdf to Archive', 'move report.pdf to Archive then resize photo.png to 100 px'),
        ]:
            self.assertFalse(semantic_safe(source, rewrite), (source, rewrite))
        self.assertTrue(semantic_safe('shift files over 2 GB to Archive', 'move files larger than 2048 MB to Archive'))
        self.assertTrue(semantic_safe('move "email.pdf" to Documents', 'move "email.pdf" to Documents'))


if __name__ == '__main__':
    unittest.main()
