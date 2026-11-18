import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from parser_model_infer import main, validate_inference_inputs


class InferencePolicyTests(unittest.TestCase):
    def test_only_current_grammar_misses_can_reach_inference(self):
        good={"id":"grammar","got":"!","parse_status":"grammar_miss","fallback_eligible":True}
        validate_inference_inputs([good])
        for bad in [
            {"id":"hard","got":"!","parse_status":"hard_refusal","fallback_eligible":False},
            {"id":"schedule","got":"!","parse_status":"schedule_refusal","fallback_eligible":False},
            {"id":"accepted","got":{"intent":"list"},"parse_status":"accepted","fallback_eligible":False},
            {"id":"forged","got":"!","parse_status":"hard_refusal","fallback_eligible":True},
            {"id":"historical","got":"!","fallback_eligible":True},
        ]:
            with self.subTest(row=bad), self.assertRaises(ValueError):
                validate_inference_inputs([good,bad])

    def test_rejected_input_never_loads_a_model_even_with_forged_input_flags(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            inputs=root/'input.jsonl'
            inputs.write_text(json.dumps({"id":"hard","text":"upload notes.pdf", "fallback_eligible":True,"parse_status":"grammar_miss"})+'\n')
            hard={"id":"hard","got":"!","parse_status":"hard_refusal","fallback_eligible":False}
            argv=['infer','--input',str(inputs),'--output',str(root/'out'),'--model',str(root/'missing-model')]
            with patch.object(sys,'argv',argv), patch('parser_model_infer.rust_score',return_value=[hard]) as score:
                with patch.dict(sys.modules,{'torch':None,'transformers':None}), self.assertRaisesRegex(ValueError,'Inference blocked'):
                    main()
                self.assertEqual(score.call_args.args[0][0]['rewrite'],'upload notes.pdf')
            self.assertFalse((root/'out/predictions.jsonl').exists())


if __name__=='__main__':
    unittest.main()
