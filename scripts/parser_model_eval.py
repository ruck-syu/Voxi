"""Reproducible parser/model evaluation. Large inputs and outputs stay outside Git.

Use --help for commands. Labels always come from the preserved inputs, never
from the current parser. Existing run directories cannot be overwritten.
"""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "app/src-tauri"
FIXED_DATE = "2026-10-28"
BANKS = ("bank-2", "bank-3", "heldout-1", "heldout-2")
READ_ONLY = {"list", "count", "size", "reveal", "duplicates", "recent_actions"}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(folder):
    return {str(p.relative_to(folder)): {"bytes": p.stat().st_size, "sha256": digest(p)}
            for p in sorted(folder.rglob("*")) if p.is_file()}


def read_rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def write_rows(path, rows):
    path.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))


def git(*args):
    return subprocess.check_output(["rtk", "proxy", "git", *args], cwd=ROOT)


def provenance(folder):
    folder.mkdir()
    (folder / "working.diff").write_bytes(git("diff", "HEAD", "--binary"))
    (folder / "status.txt").write_bytes(git("status", "--short"))
    files = git("ls-files", "--cached", "--others", "--exclude-standard", "-z").decode().split("\0")
    for name in filter(None, files):
        source = ROOT / name
        if source.is_file():
            dest = folder / "worktree" / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, dest)
    return {"git_head": git("rev-parse", "HEAD").decode().strip(),
            "git_branch": git("branch", "--show-current").decode().strip(),
            "captured_utc": datetime.now(timezone.utc).isoformat(),
            "host": platform.platform(), "machine": platform.machine(),
            "python": sys.version, "python_executable": sys.executable,
            "rustc": subprocess.check_output(["rtk", "proxy", "rustc", "--version"], text=True).strip(),
            "cargo": subprocess.check_output(["rtk", "proxy", "cargo", "--version"], text=True).strip(),
            "evaluation_date": FIXED_DATE, "files": inventory(folder)}


def preserve(args):
    source, dest = args.source.resolve(), args.experiment.resolve()
    if dest == source or source in dest.parents:
        raise ValueError("Experiment must be outside the temporary source")
    dest.mkdir(parents=True, exist_ok=False)
    shutil.copytree(source, dest / "legacy", ignore=shutil.ignore_patterns(".venv", "__pycache__"))
    meta = provenance(dest / "provenance")
    meta.update({"source": str(source), "experiment": str(dest),
                 "legacy_files": inventory(dest / "legacy"),
                 "inference_settings": {"max_input_tokens": 64, "max_new_tokens": 64,
                                        "num_beams": 1, "do_sample": False,
                                        "decode_cleanup": False},
                 "legacy_timings": "Kaggle CPU int8, not a local-runtime measurement"})
    python = source / ".venv/bin/python"
    if python.exists():
        result = subprocess.run([str(python), "-m", "pip", "freeze"], capture_output=True, text=True)
        # uv-created environments need not include pip.
        if result.returncode:
            result = subprocess.run([str(python), "-c",
                "import importlib.metadata as m; print('\\n'.join(sorted(d.metadata['Name']+'=='+d.version for d in m.distributions())))"],
                capture_output=True, text=True, check=True)
        (dest / "requirements-legacy.txt").write_text(result.stdout)
        meta["legacy_python"] = str(python)
    write_json(dest / "manifest.json", meta)
    print(f"Preserved {len(meta['legacy_files'])} files in {dest}")


def verify_legacy(experiment):
    manifest = json.loads((experiment / "manifest.json").read_text())
    for name, expected in manifest["legacy_files"].items():
        path = experiment / "legacy" / name
        if not path.is_file() or digest(path) != expected["sha256"]:
            raise ValueError(f"Preserved artifact changed: {name}")


def rust_score(rows, folder):
    folder.mkdir()
    write_rows(folder / "rewrites.jsonl", rows)
    result = subprocess.run(["rtk", "proxy", "cargo", "test", "--lib", "feas_score", "--", "--ignored", "--nocapture"],
                            cwd=CRATE, env={**os.environ, "FEAS_DIR": str(folder)}, capture_output=True, text=True)
    (folder / "cargo.log").write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f"Rust scoring failed; see {folder / 'cargo.log'}")
    scored = read_rows(folder / "scored.jsonl")
    if len(scored) != len(rows) or any(a["id"] != b["id"] for a, b in zip(rows, scored)):
        raise ValueError("Rust scoring returned incomplete or reordered rows")
    return scored


def load_inputs(legacy):
    seeds = {s["id"]: s for s in json.loads((legacy / "seeds.json").read_text())}
    for i, row in enumerate(read_rows(legacy / "gen/clean.jsonl")):
        want = "!" if row["tgt"] == "REFUSE" else seeds[row["id"]]["want"]
        if want != "!" and seeds[row["id"]]["canonical"] != row["tgt"]:
            raise ValueError(f"Generated target disagrees with preserved seed: {i}")
        yield {"id": f"generated-development:{i}", "set": "generated-development", "split": "development",
               "at": row["origin"], "text": row["src"], "want": want, "target": row["tgt"],
               "label_source": f"gen/clean.jsonl:{i + 1}"}
    corpus_test = {(r["at"], r["text"]) for r in read_rows(legacy / "kaggle-data-v3/eval.jsonl") if r["set"] == "corpus-test"}
    for name in ("corpus", *BANKS):
        for i, row in enumerate(read_rows(legacy / f"{name}.jsonl")):
            dataset = "corpus-test" if name == "corpus" and (row["at"], row["text"]) in corpus_test else name
            yield {"id": f"{name}:{i}", "set": dataset,
                   "split": "regression" if name == "corpus" and dataset != "corpus-test" else "previously-inspected-evaluation",
                   "at": row["at"], "text": row["text"], "want": row["want"],
                   "historical_rules": row.get("rules"), "label_source": f"{name}.jsonl:{i + 1}"}


def category(got, want):
    if want == "!":
        return "correct_refusal" if got == "!" else "accepted_refuse_label"
    if got == "!":
        return "refused_supported"
    return "correct_intent" if got == want else "accepted_misread"


def summarize(rows, key):
    counts = defaultdict(Counter)
    for row in rows:
        c = counts[row["set"]]
        c["total"] += 1
        kind = category(row[key], row["want"])
        c[kind] += 1
        if kind in ("accepted_misread", "accepted_refuse_label"):
            scope = "read_only" if row[key]["intent"] in READ_ONLY else "potentially_mutating"
            c[f"{kind}_{scope}"] += 1
    return dict(counts)


def fallback_eligible(row):
    """Only an authoritative Rust grammar miss permits model inference.

    Old snapshots without the typed policy must be refreshed. A caller-supplied
    boolean cannot turn a hard refusal or scheduler restriction into eligibility.
    """
    status = row.get("parse_status")
    if status not in ("accepted", "grammar_miss", "hard_refusal", "schedule_refusal"):
        raise ValueError("Missing Rust parse_status; refresh rule outcomes before fallback")
    if (status == "accepted") != (row["rules"] != "!"):
        raise ValueError("Rule outcome disagrees with Rust parse_status")
    eligible = status == "grammar_miss" and not row.get("blocked_before_fallback", False)
    if "fallback_eligible" in row and row["fallback_eligible"] is not eligible:
        raise ValueError("Fallback eligibility disagrees with Rust refusal policy")
    return eligible


def rule_decision(row):
    return row["parse_status"] if row["parse_status"] in ("hard_refusal", "schedule_refusal") else "rules"


def validate_predictions(rows, predictions):
    if any(not isinstance(r.get("id"), str) for r in predictions):
        raise ValueError("Predictions need IDs from the refreshed fallback.jsonl; historical outputs cannot be reused directly")
    by_id = {r["id"]: r for r in predictions}
    expected = {r["id"] for r in rows if fallback_eligible(r)}
    if len(by_id) != len(predictions) or set(by_id) != expected:
        raise ValueError(f"Predictions must cover fallback exactly: missing {len(expected - set(by_id))}, extra {len(set(by_id) - expected)} (or duplicate IDs)")
    for r in rows:
        if fallback_eligible(r):
            prediction = by_id[r["id"]]
            if prediction["text"] != r["text"] or not isinstance(prediction.get("rewrite"), str):
                raise ValueError(f"Invalid prediction text/rewrite: {r['id']}")
    return by_id


def combine(rows, scored, safe):
    rewrites = {r["id"]: r for r in scored}
    for original in rows:
        eligible = fallback_eligible(original)
        r = dict(original, combined=original["rules"], decision=rule_decision(original))
        if eligible:
            s = rewrites[r["id"]]
            r.update(rewrite=s["rewrite"], rewrite_outcome=s["got"], guard_allowed=safe(r["text"], s["rewrite"]))
            r["rewrite_parse_status"] = s.get("parse_status")
            r["rewrite_hard_refusal_reason"] = s.get("hard_refusal_reason")
            r["combined"] = s["got"] if r["guard_allowed"] else "!"
            r["decision"] = "guard_rejection" if not r["guard_allowed"] else "model_refusal" if s["got"] == "!" else "model_accepted"
        yield r


def refresh(args):
    experiment = args.experiment.resolve()
    verify_legacy(experiment)
    run = experiment / args.run
    run.mkdir(exist_ok=False)
    meta = provenance(run / "provenance")
    inputs = list(load_inputs(experiment / "legacy"))
    scored = rust_score([{**r, "rewrite": r["text"]} for r in inputs], run / "rules-score")
    rows = [{**r, "rules": r["got"]} for r in scored]
    for row in rows:
        fallback_eligible(row)
    # Canonical targets are diagnostics only: they never replace frozen labels.
    targets = [dict(r, rewrite=r["target"]) for r in rows if "target" in r and r["want"] != "!"]
    target_scores = rust_score(targets, run / "target-score")
    conflicts = [{**r, "target_outcome": s["got"]} for r, s in zip(targets, target_scores) if s["got"] != r["want"]]
    write_rows(run / "label-conflicts.jsonl", conflicts)
    write_rows(run / "evaluation.jsonl", rows)
    fallback = [r for r in rows if fallback_eligible(r)]
    write_rows(run / "fallback.jsonl", fallback)
    historical = read_rows(experiment / "legacy/out-v3/rewrites_t5-efficient-tiny-v3.jsonl")
    old = {(r["set"], r["at"], r["text"]) for r in historical}
    meta.update({"experiment": str(experiment), "rows": len(rows), "fallback_rows": len(fallback),
                 "frozen_label_target_conflicts": len(conflicts), "rules": summarize(rows, "rules"),
                 "rule_decisions": dict(Counter(r["parse_status"] for r in rows)),
                 "hard_refusal_reasons": dict(Counter(r["hard_refusal_reason"] for r in rows if r["parse_status"] == "hard_refusal")),
                 "fallback_without_historical_prediction": dict(Counter(r["set"] for r in fallback if (r["set"], r["at"], r["text"]) not in old)),
                 "previous_predictions_no_longer_eligible": sum((r["set"], r["at"], r["text"]) in old and not r["fallback_eligible"] for r in rows),
                 "artifacts": {p.name: {"sha256": digest(p), "bytes": p.stat().st_size} for p in run.glob("*.jsonl")}})
    write_json(run / "manifest.json", meta)
    print(json.dumps({k: meta[k] for k in ("rows", "fallback_rows", "frozen_label_target_conflicts", "fallback_without_historical_prediction", "previous_predictions_no_longer_eligible", "rules")}, indent=2))


def compare(args):
    experiment = args.experiment.resolve()
    verify_legacy(experiment)
    run = experiment / args.run
    rows = read_rows(run / "evaluation.jsonl")
    manifest = json.loads((run / "manifest.json").read_text())
    evaluator = ROOT / "scripts/parser_model_eval.py"
    frozen_evaluator = manifest["files"].get("worktree/scripts/parser_model_eval.py", {})
    if digest(evaluator) != frozen_evaluator.get("sha256"):
        raise ValueError("Evaluator changed since refresh; create a fresh run")
    if digest(run / "evaluation.jsonl") != manifest["artifacts"]["evaluation.jsonl"]["sha256"]:
        raise ValueError("Refreshed evaluation changed")
    # Never allow scores from a different parser revision/working tree.
    current = {str(p.relative_to(ROOT)): digest(p) for p in (CRATE / "src").rglob("*.rs")}
    frozen = {k.removeprefix("worktree/"): v["sha256"] for k, v in manifest["files"].items() if k.startswith("worktree/app/src-tauri/src/") and k.endswith(".rs")}
    if current != frozen:
        raise ValueError("Rust sources changed since refresh; create a fresh run")
    for name in ("Cargo.toml", "Cargo.lock", "build.rs"):
        source = CRATE / name
        saved = run / "provenance/worktree/app/src-tauri" / name
        if source.exists() != saved.exists() or (source.exists() and digest(source) != digest(saved)):
            raise ValueError(f"{name} changed since refresh; create a fresh run")
    predictions = read_rows(args.predictions)
    by_id = validate_predictions(rows, predictions)
    prediction_manifest = args.prediction_manifest or args.predictions.parent / "manifest.json"
    inference = json.loads(prediction_manifest.read_text())
    runner = prediction_manifest.parent / "inference-script.py"
    if not runner.is_file() or digest(runner) != inference["script_sha256"]:
        raise ValueError("Archived inference runner differs from inference manifest")
    for name, expected in inference.get("helper_hashes", {}).items():
        if digest(prediction_manifest.parent / name) != expected:
            raise ValueError(f"Archived inference helper changed: {name}")
    if inference["predictions_sha256"] != digest(args.predictions):
        raise ValueError("Prediction hash differs from inference manifest")
    if inference["input_sha256"] != digest(run / "fallback.jsonl"):
        raise ValueError("Inference did not use the refreshed fallback inputs")
    if inventory(Path(inference["model"])) != inference["model_files"]:
        raise ValueError("Inference checkpoint/tokenizer changed")
    output = run / args.output
    output.mkdir(exist_ok=False)
    spec = importlib.util.spec_from_file_location("preserved_guard", experiment / "legacy/guard.py")
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    scored = rust_score([{**r, "rewrite": by_id[r["id"]]["rewrite"]} for r in rows if fallback_eligible(r)], output / "rewrite-score")
    rows = list(combine(rows, scored, guard.safe))
    write_rows(output / "outcomes.jsonl", rows)
    write_json(output / "summary.json", {"evaluation_sha256": digest(run / "evaluation.jsonl"),
        "predictions_sha256": digest(args.predictions), "guard_sha256": digest(experiment / "legacy/guard.py"),
        "prediction_manifest_sha256": digest(prediction_manifest), "inference": inference,
        "rules": summarize(rows, "rules"), "combined": summarize(rows, "combined"),
        "decisions": {name: dict(Counter(r["decision"] for r in rows if r["set"] == name)) for name in sorted({r["set"] for r in rows})},
        "note": "Parser acceptance is not planner permission. Banks were previously inspected; these are not pristine holdouts."})
    print(f"Wrote {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name, action in (("preserve", preserve), ("refresh", refresh), ("compare", compare)):
        p = commands.add_parser(name)
        p.add_argument("--experiment", type=Path, required=True)
        p.set_defaults(action=action)
        if name == "preserve":
            p.add_argument("--source", type=Path, required=True)
        else:
            p.add_argument("--run", default="refresh-001")
        if name == "compare":
            p.add_argument("--predictions", type=Path, required=True)
            p.add_argument("--prediction-manifest", type=Path)
            p.add_argument("--output", default="comparison-v3")
    args = parser.parse_args()
    args.action(args)


if __name__ == "__main__":
    main()
