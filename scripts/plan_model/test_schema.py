import copy
import unittest
from schema import decode, encode


class SchemaTests(unittest.TestCase):
    def setUp(self):
        self.plan = {"steps": [["mkdir", "R&D 東京", "Downloads"], ["move", "pdfs", "Downloads", {"ref": 0}]], "schedule": ["repeat", 1, "days", "21:00"]}

    def test_roundtrip_keeps_literal_values_and_reference(self):
        self.assertEqual(decode(encode(self.plan)), self.plan)
        self.assertEqual(decode("REFUSE"), "REFUSE")

    def test_schema_refuses_bad_references_invented_actions_and_bad_schedules(self):
        for reference in [-1, 1, 8, True, "0"]:
            value = copy.deepcopy(self.plan)
            value["steps"][1][3] = {"ref": reference}
            with self.assertRaises(ValueError): encode(value)
        for schedule in [["repeat", 5, "minutes", None], ["repeat", 2, "hours", "21:00"], ["repeat", 1, "days", "25:00"], ["at", "21:00", "someday"], ["weekday", "funday", "09:00"]]:
            value = copy.deepcopy(self.plan)
            value["schedule"] = schedule
            with self.assertRaises(ValueError): encode(value)
        value = copy.deepcopy(self.plan)
        value["steps"][1][0] = "upload"
        with self.assertRaises(ValueError): encode(value)
        with self.assertRaises(ValueError): decode('{"steps":[],"schedule":null}')


if __name__ == "__main__": unittest.main()
