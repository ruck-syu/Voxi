import unittest
from literals import bind, restore, substitute_target


class LiteralTests(unittest.TestCase):
    def test_exact_unicode_and_apostrophes_survive_generation(self):
        text = 'make "資料 2028" in Documents and rename "King’s notes.txt" to "R&D.txt"'
        encoded, bindings = bind(text)
        self.assertNotIn('資料', encoded)
        target = {'steps':[['mkdir','資料 2028','Documents'],['rename','King’s notes.txt','Documents','R&D.txt']], 'schedule':None}
        self.assertEqual(restore(substitute_target(target, bindings), bindings), target)
        self.assertEqual(bind("copy King's report.pdf to Documents")[1], {})

    def test_missing_or_invented_aliases_fail_closed(self):
        _, bindings = bind('make "資料" in Documents')
        for value in [{'name':'invented'}, {'name':'~9~'}, {'name':'prefix ~0~'}]:
            with self.assertRaises(ValueError): restore(value, bindings)
        self.assertEqual(restore('REFUSE', bindings), 'REFUSE')

    def test_aliases_do_not_collide_with_source_and_malformed_quotes_refuse(self):
        _, bindings = bind('make "~0~" in Documents')
        self.assertEqual(bindings, {'~1~':'~0~'})
        self.assertEqual(restore({'name':'~1~'}, bindings), {'name':'~0~'})
        with self.assertRaises(ValueError): bind('make "unclosed')

    def test_literal_operation_word_does_not_replace_an_operation_or_type(self):
        _, bindings = bind('make "move" then copy "pdfs"')
        target = {'steps':[['mkdir','move','Downloads'],['copy','pdfs','Downloads','pdfs']], 'schedule':None}
        bound = substitute_target(target, bindings)
        self.assertEqual(bound['steps'][0][0], 'mkdir')
        self.assertEqual(bound['steps'][1][:2], ['copy','pdfs'])
        self.assertEqual(restore(bound,bindings),target)

    def test_sentencepiece_spaces_only_normalize_complete_virtual_aliases(self):
        _, bindings = bind('make "  資料  " in Documents')
        self.assertEqual(restore({'name':'~ 0~ '}, bindings), {'name':'  資料  '})
        for value in ['prefix ~ 0~ ', '~ 9~ ', '~ 00~ ']:
            with self.assertRaises(ValueError): restore({'name':value}, bindings)


if __name__ == '__main__': unittest.main()
