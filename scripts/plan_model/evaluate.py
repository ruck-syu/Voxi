"""Audit saved Kaggle predictions; schema validity is never semantic accuracy."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
from schema import decode, encode
from literals import bind, restore


def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def score(rows):
    counts = Counter()
    errors = []
    for row in rows:
        expected = row["target"]
        encode(expected)
        counts["rows"] += 1
        counts["refusal_requests" if expected == "REFUSE" else "supported_requests"] += 1
        try:
            actual = decode(row["prediction"])
        except (ValueError, TypeError, KeyError) as error:
            actual = None
            counts["invalid_schema"] += 1
            errors.append({"id": row["id"], "kind": "invalid_schema", "message": str(error)})
        if actual == expected:
            counts["exact"] += 1
            if expected != "REFUSE": counts["correct_supported"] += 1
        elif isinstance(actual, dict):
            counts["accepted_wrong_plan"] += 1
            kind = "accepted_refusal" if expected == "REFUSE" else "wrong_plan"
            counts[kind] += 1
            errors.append({"id": row["id"], "kind": kind, "text": row["text"], "expected": expected, "actual": actual})
        elif actual == "REFUSE" and expected != "REFUSE":
            counts["refused_supported"] += 1
            errors.append({"id": row["id"], "kind": "refused_supported", "text": row["text"]})
    return {"counts": dict(counts), "errors": errors,
            "semantic_gate": "PASS" if counts["supported_requests"] and counts["accepted_wrong_plan"] == 0 and counts["correct_supported"] / counts["supported_requests"] >= .9 else "FAIL",
            "gate_definition": "zero schema-valid wrong plans or refusal violations and at least 90% exact supported recovery; export/runtime/planner checks remain separate"}


def restore_predictions(rows):
    result = []
    for row in rows:
        try:
            if row.get('schema_error') == 'generation did not terminate': raise ValueError('generation did not terminate')
            parsed = restore(decode(row['raw_prediction']), bind(row['text'])[1])
            normalized, error = encode(parsed), None
        except (ValueError, TypeError, KeyError) as exc:
            parsed, normalized, error = None, 'INVALID_MODEL_OUTPUT', str(exc)
        result.append({**row, 'prediction':normalized, 'parsed':parsed, 'schema_error':error})
    return result


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--restore-literals", action="store_true", help="Replay frozen raw predictions with the current literal codec; never run training")
    args = ap.parse_args()
    rows = [json.loads(line) for line in args.input.read_text().splitlines() if line.strip()]
    if args.restore_literals: rows = restore_predictions(rows)
    result = score(rows)
    result["predictions_sha256"] = sha(args.input)
    result["literal_codec_replay"] = args.restore_literals
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({key: value for key,value in result.items() if key != "errors"}, indent=2))
