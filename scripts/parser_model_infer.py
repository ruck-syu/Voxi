"""Run preserved seq2seq checkpoints locally, recording exact input/model hashes.

This is an experiment runtime, not inference integrated into the Voxi app.
"""
import argparse
from datetime import datetime, timezone
import importlib.metadata
import json
import platform
import shutil
from pathlib import Path
import statistics
import sys
import time

from parser_model_eval import digest, inventory, read_rows, write_json, write_rows, rust_score, fallback_eligible


def validate_inference_inputs(scored):
    if not scored:
        raise ValueError("No grammar misses to send to inference")
    rejected = [row["id"] for row in scored if not fallback_eligible({**row, "rules": row["got"]})]
    if rejected:
        raise ValueError(f"Inference blocked by Rust refusal policy for {len(rejected)} inputs ({rejected[:5]}); rebuild fallback inputs")


def percentile(values, p):
    values = sorted(values)
    return values[min(len(values) - 1, int((len(values) - 1) * p))]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--model", type=Path, required=True)
    p.add_argument("--input", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--precision", choices=("fp32", "int8"), default="fp32")
    p.add_argument("--batch-size", type=int, default=32)
    p.add_argument("--threads", type=int, default=2)
    p.add_argument("--benchmark", type=int, default=40)
    p.add_argument("--slots", action="store_true")
    p.add_argument("--semantic-guard", action="store_true")
    args = p.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(Path(__file__), output / "inference-script.py")
    from parser_slots import substitute, semantic_safe
    helpers = {}
    for name in ("parser_model_eval.py", "parser_slots.py"):
        source = Path(__file__).parent / name
        shutil.copy2(source, output / name)
        helpers[name] = digest(source)
    rows = read_rows(args.input)
    policy = rust_score([{**row, "rewrite": row["text"]} for row in rows], output / "policy-score")
    validate_inference_inputs(policy)
    # Load no checkpoint and execute no generation until Rust authorizes every
    # input. Re-scoring prevents stale/forged JSON flags bypassing hard refusals.
    import torch
    from transformers import AutoModelForSeq2SeqLM, AutoTokenizer
    torch.set_num_threads(args.threads)
    torch.manual_seed(0)
    tokenizer = AutoTokenizer.from_pretrained(args.model, local_files_only=True)
    model = AutoModelForSeq2SeqLM.from_pretrained(args.model, local_files_only=True).cpu().eval()
    if args.precision == "int8":
        # Apple ARM wheels expose qnnpack but may leave the engine as NoQEngine.
        engines = torch.backends.quantized.supported_engines
        if torch.backends.quantized.engine not in engines:
            torch.backends.quantized.engine = "qnnpack" if "qnnpack" in engines else engines[0]
        model = torch.ao.quantization.quantize_dynamic(model, {torch.nn.Linear}, dtype=torch.qint8)
        torch.save(model.state_dict(), output / "quantized-state.pt")
    settings = {"max_input_tokens": 64, "max_new_tokens": 64, "num_beams": 1, "do_sample": False}

    def generate(texts):
        tokens = tokenizer(texts, max_length=64, truncation=True, padding=True, return_tensors="pt")
        with torch.inference_mode():
            generated = model.generate(**tokens, max_new_tokens=64, num_beams=1, do_sample=False)
        return tokenizer.batch_decode(generated, skip_special_tokens=True, clean_up_tokenization_spaces=False)

    def rewrite(texts):
        bundles, inputs, failures = [], [], []
        for text in texts:
            try:
                bundle = substitute(text) if args.slots else None
                bundles.append(bundle)
                inputs.append(bundle.encoded if bundle else text)
                failures.append(None)
            except ValueError as error:
                bundles.append(None)
                inputs.append("REFUSE")
                failures.append(str(error))
        records = []
        for text, bundle, model_input, failure, raw in zip(texts, bundles, inputs, failures, generate(inputs)):
            restored = raw.strip()
            if bundle is not None:
                try:
                    restored = bundle.restore(restored)
                except ValueError as error:
                    failure = str(error)
            if failure:
                restored = "REFUSE"
            semantic_rejection = args.semantic_guard and not semantic_safe(text, restored)
            records.append({"rewrite": "REFUSE" if semantic_rejection else restored,
                            "raw_rewrite": raw.strip(), "model_input": model_input,
                            "slot_rejection": failure, "semantic_rejection": bool(semantic_rejection)})
        return records

    t0 = time.perf_counter()
    predictions = []
    for start in range(0, len(rows), args.batch_size):
        batch = rows[start:start + args.batch_size]
        rewrites = rewrite([r["text"] for r in batch])
        predictions.extend({"id": r["id"], "text": r["text"], **result}
                           for r, result in zip(batch, rewrites))
        if start % (args.batch_size * 20) == 0:
            print(f"generated {len(predictions)}/{len(rows)} in {time.perf_counter() - t0:.1f}s", flush=True)
    elapsed = time.perf_counter() - t0
    write_rows(output / "predictions.jsonl", predictions)
    # Deterministic spread across the full input list, rather than only its first bank.
    count = min(args.benchmark, len(rows))
    samples = [rows[i * len(rows) // count]["text"] for i in range(count)] if count else []
    for text in samples[:3]:
        rewrite([text])
    latencies = []
    for text in samples:
        start = time.perf_counter()
        rewrite([text])
        latencies.append(1000 * (time.perf_counter() - start))
    model_files = inventory(args.model)
    write_json(output / "manifest.json", {
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "input": str(args.input.resolve()), "input_sha256": digest(args.input),
        "predictions_sha256": digest(output / "predictions.jsonl"), "model": str(args.model.resolve()),
        "model_files": model_files, "model_bytes": sum(r["bytes"] for r in model_files.values()),
        "quantized_state_bytes": (output / "quantized-state.pt").stat().st_size if args.precision == "int8" else None,
        "script_sha256": digest(Path(__file__)), "runtime": "PyTorch CPU experiment; not Rust app integration",
        "helper_hashes": helpers, "preprocessing": {"slots": args.slots, "semantic_guard": args.semantic_guard},
        "policy_score_sha256": digest(output / "policy-score/scored.jsonl"),
        "precision": args.precision, "threads": args.threads, "batch_size": args.batch_size,
        "quantization_engine": torch.backends.quantized.engine if args.precision == "int8" else None,
        "python": sys.version, "platform": platform.platform(), "machine": platform.machine(),
        "packages": {name: importlib.metadata.version(name) for name in ("torch", "transformers", "tokenizers", "safetensors")},
        "generation_settings": settings, "rows": len(rows), "total_batch_seconds": elapsed,
        "truncated_inputs": sum(len(tokenizer.encode(r["text"])) > 64 for r in rows),
        "single_sentence_benchmark": {"samples": count, "warmup": min(3, count), "timing_includes": "configured slot/semantic processing, tokenization, greedy generation, decoding",
            "median_ms": statistics.median(latencies) if count else None,
            "p95_ms": percentile(latencies, .95) if count else None, "all_ms": latencies},
    })
    print(f"Saved {len(predictions)} predictions to {output}", flush=True)


if __name__ == "__main__":
    main()
