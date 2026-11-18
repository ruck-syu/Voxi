import unittest
from pathlib import Path
import tempfile
from parser_fresh_eval import finish_row, gates, verify_inference
from parser_model_eval import digest, inventory, write_json


class FreshEvaluationTests(unittest.TestCase):
    def test_scheduler_refusals_and_accepted_rules_never_reach_model(self):
        for rule, status in [('!','hard_refusal'), ('!','schedule_refusal'), ({'intent':'trash'},'accepted')]:
            row={'text':'x','rules':rule,'fallback_eligible':False,'parse_status':status}
            self.assertEqual(finish_row(row,None,None,lambda *_:self.fail('guard ran'))['combined'],rule)

    def test_changed_schedule_is_rejected_without_changing_label(self):
        row={'text':'x','rules':'!','want':{'intent':'list','scheduled':True},'fallback_eligible':True,
             'schedule_due':'12:05','schedule_rule':'None'}
        row['parse_status']='grammar_miss'
        prediction={'rewrite':'list files in 5 hours'}
        scored={'got':{'intent':'list','scheduled':True},'schedule_due':'17:00','schedule_rule':'None'}
        result=finish_row(row,scored,prediction,lambda *_:True)
        self.assertEqual(result['decision'],'schedule_guard_rejection')
        self.assertEqual(result['combined'],'!')
        self.assertEqual(result['want'],row['want'])

    def test_safety_and_package_gates_cannot_be_hidden_by_coverage(self):
        rows=[{'want':'!','combined':{'intent':'trash'},'rules':'!','fallback_eligible':True,'parse_status':'grammar_miss'},
              {'want':{'intent':'list'},'combined':{'intent':'list'},'rules':'!','fallback_eligible':True,'parse_status':'grammar_miss'}]
        result=gates(rows,{'model_bytes':64_000_000,'single_sentence_benchmark':{'p95_ms':10}})
        self.assertEqual(result['recovery_fraction'],1)
        self.assertFalse(result['checks']['no_refuse_violations'])
        self.assertFalse(result['checks']['model_and_tokenizer_at_most_25_mb'])
        self.assertFalse(result['qualifies_for_integration'])

    def test_runner_and_prediction_provenance_cannot_be_substituted(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp)
            model=folder/'model'
            model.mkdir()
            (model/'weights').write_text('original weights')
            inputs=folder/'inputs.jsonl'
            inputs.write_text('input')
            (folder/'predictions.jsonl').write_text('prediction')
            (folder/'inference-script.py').write_text('runner')
            write_json(folder/'manifest.json',{
                'predictions_sha256':digest(folder/'predictions.jsonl'),
                'input_sha256':digest(inputs),'script_sha256':digest(folder/'inference-script.py'),
                'model':str(model),'model_files':inventory(model),'helper_hashes':{}})
            verify_inference(folder,inputs,inventory(model))
            (folder/'inference-script.py').write_text('substituted runner')
            with self.assertRaises(ValueError):
                verify_inference(folder,inputs,inventory(model))


if __name__=='__main__':
    unittest.main()
