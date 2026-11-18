"""Checks that evaluation cannot hide errors through labels or fallback routing."""
from pathlib import Path
import tempfile
import unittest

from parser_model_eval import combine, load_inputs, summarize, validate_predictions, write_json, write_rows, fallback_eligible


class EvaluationTests(unittest.TestCase):
    def test_rules_acceptance_wins_even_when_rules_are_wrong(self):
        row = {"id": "x", "set": "dev", "rules": {"intent": "trash"},
               "want": {"intent": "list"}, "fallback_eligible": False, "parse_status": "accepted"}
        result = list(combine([row], [], lambda *_: self.fail("Guard must not run")))
        self.assertEqual(result[0]["combined"], {"intent": "trash"})
        self.assertEqual(summarize(result, "combined")["dev"]["accepted_misread_potentially_mutating"], 1)

    def test_rejections_and_unsafe_rewrites_remain_visible(self):
        rows = [{"id": str(i), "set": "dev", "text": text, "rules": "!",
                 "want": want, "fallback_eligible": True, "parse_status": "grammar_miss"}
                for i, (text, want) in enumerate([
                    ("reject", {"intent": "list"}), ("unsafe", "!"),
                    ("refuse", {"intent": "list"}), ("correct", {"intent": "list"})])]
        scored = [{"id": str(i), "rewrite": "REFUSE" if i == 2 else "list files",
                   "got": "!" if i == 2 else {"intent": "list"}} for i in range(4)]
        result = list(combine(rows, scored, lambda text, _: text != "reject"))
        self.assertEqual([r["decision"] for r in result],
                         ["guard_rejection", "model_accepted", "model_refusal", "model_accepted"])
        self.assertEqual(result[0]["rewrite_outcome"], {"intent": "list"})
        counts = summarize(result, "combined")["dev"]
        self.assertEqual(counts["refused_supported"], 2)
        self.assertEqual(counts["accepted_refuse_label_read_only"], 1)
        self.assertEqual(counts["correct_intent"], 1)

    def test_prediction_coverage_rejects_missing_extra_duplicates_and_text_changes(self):
        rows = [{"id": "newly-refused", "text": "original", "rules": "!", "fallback_eligible": True, "parse_status": "grammar_miss"}]
        good = {"id": "newly-refused", "text": "original", "rewrite": "REFUSE"}
        for bad in ([], [{"text": "original", "rewrite": "REFUSE"}], [good, good], [dict(good, id="old-row")], [dict(good, text="changed")], [dict(good, rewrite=None)]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                validate_predictions(rows, bad)
        self.assertEqual(validate_predictions(rows, [good]), {"newly-refused": good})

    def test_hard_refusals_are_terminal_and_cannot_be_made_eligible_by_json(self):
        hard = {"id":"hard", "set":"dev", "text":"upload notes.pdf", "want":"!", "rules":"!",
                "parse_status":"hard_refusal", "hard_refusal_reason":"unsupported_action", "fallback_eligible":False}
        result = list(combine([hard], [], lambda *_: self.fail("Guard/model path must not run")))
        self.assertEqual(result[0]["combined"], "!")
        self.assertEqual(result[0]["decision"], "hard_refusal")
        self.assertEqual(validate_predictions([hard], []), {})
        with self.assertRaises(ValueError):
            validate_predictions([hard], [{"id":"hard","text":hard["text"],"rewrite":"copy notes.pdf to Documents"}])
        with self.assertRaises(ValueError):
            fallback_eligible(dict(hard, fallback_eligible=True))
        with self.assertRaises(ValueError):
            fallback_eligible({"rules":"!", "fallback_eligible":True})

    def test_a_model_cannot_introduce_a_hard_refused_operation(self):
        original={"id":"grammar", "set":"dev", "text":"unfamiliar wording", "want":"!", "rules":"!",
                  "parse_status":"grammar_miss", "fallback_eligible":True}
        rewrite={"id":"grammar", "rewrite":"upload notes.pdf to Drive", "got":"!",
                 "parse_status":"hard_refusal", "hard_refusal_reason":"unsupported_action"}
        result=list(combine([original],[rewrite],lambda *_:True))[0]
        self.assertEqual(result['combined'],'!')
        self.assertEqual(result['rewrite_parse_status'],'hard_refusal')
        self.assertEqual(result['rewrite_hard_refusal_reason'],'unsupported_action')

    def test_inputs_preserve_frozen_labels_and_ignore_historical_rules(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "gen").mkdir()
            (root / "kaggle-data-v3").mkdir()
            want = {"intent": "list", "src": {"kind": "item", "name": "猫.pdf"}}
            write_json(root / "seeds.json", [{"id": "s1", "canonical": "list 猫.pdf", "want": want}])
            write_rows(root / "gen/clean.jsonl", [{"src": "show 猫.pdf", "tgt": "list 猫.pdf", "id": "s1", "origin": "dev"},
                                                 {"src": "unsupported", "tgt": "REFUSE", "origin": "dev"}])
            write_rows(root / "kaggle-data-v3/eval.jsonl", [])
            for name in ("corpus", "bank-2", "bank-3", "heldout-1", "heldout-2"):
                write_rows(root / f"{name}.jsonl", [{"text": "show 猫.pdf", "at": "cases:1", "want": want, "rules": "historically-wrong"}])
            rows = list(load_inputs(root))
            self.assertEqual(rows[0]["want"], want)
            self.assertEqual(rows[1]["want"], "!")
            self.assertTrue(all("rules" not in r for r in rows))
            self.assertTrue(all(r["split"] != "development" for r in rows[2:]))


if __name__ == "__main__":
    unittest.main()
