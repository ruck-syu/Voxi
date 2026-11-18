import unittest
from evaluate import score, restore_predictions
from schema import encode


class EvaluationTests(unittest.TestCase):
    def test_literal_restoration_does_not_repair_a_wrong_action_or_schedule(self):
        target = {'steps':[['mkdir','Badger','Downloads'],['move','pdfs','Downloads',{'ref':0}]],'schedule':['after',30,'minutes']}
        wrong = {'steps':[['mkdir','~0~','Documents'],['copy','pdfs','Downloads',{'ref':0}]],'schedule':['after',30,'hours']}
        rows = restore_predictions([{'id':'x','text':'make "Badger" in Downloads then move PDFs into it in 30 minutes','target':target,'raw_prediction':encode(wrong)}])
        result = score(rows)
        self.assertEqual(rows[0]['parsed']['steps'][0][1], 'Badger')
        self.assertEqual(rows[0]['parsed']['steps'][1][0], 'copy')
        self.assertEqual(result['counts']['accepted_wrong_plan'],1)
        self.assertEqual(result['semantic_gate'],'FAIL')

    def test_wrong_direction_and_discarded_schedules_are_errors(self):
        target = {"steps": [["move", "pdfs", "Downloads", "Documents"]], "schedule": ["repeat", 1, "days", None]}
        wrong = {"steps": [["move", "pdfs", "Documents", "Downloads"]], "schedule": None}
        result = score([{"id": "a", "text": "move daily", "target": target, "prediction": encode(wrong)}])
        self.assertEqual(result["counts"]["accepted_wrong_plan"], 1)
        self.assertEqual(result["semantic_gate"], "FAIL")

    def test_refusal_violations_and_malformed_outputs_cannot_pass(self):
        value = {"steps": [["move", "files", "Downloads", "Documents"]], "schedule": None}
        result = score([{"id": "a", "text": "unsupported", "target": "REFUSE", "prediction": encode(value)},
                        {"id": "b", "text": "unsupported", "target": "REFUSE", "prediction": "not json"}])
        self.assertEqual(result["counts"]["accepted_refusal"], 1)
        self.assertEqual(result["counts"]["invalid_schema"], 1)
        self.assertEqual(result["semantic_gate"], "FAIL")


if __name__ == "__main__": unittest.main()
