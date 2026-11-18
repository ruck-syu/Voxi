"""Check downloaded Kaggle weights and CPU inference without any training."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import time
from evaluate import score, restore_predictions
from schema import decode
from schema import encode
from literals import bind, restore


def main(args):
    import torch
    from transformers import AutoModelForSeq2SeqLM, AutoTokenizer
    torch.set_num_threads(1)
    torch.manual_seed(417)
    manifest = json.loads((args.artifacts / "run-manifest.json").read_text())
    model_dir = args.artifacts / "model-voxi-plan-v4"
    for name, expected in manifest["model_files"].items():
        actual = hashlib.sha256((model_dir/name).read_bytes()).hexdigest()
        if actual != expected: raise ValueError(f"downloaded model hash mismatch: {name}")
    tokenizer = AutoTokenizer.from_pretrained(model_dir, local_files_only=True)
    model = AutoModelForSeq2SeqLM.from_pretrained(model_dir, local_files_only=True).cpu().eval()
    model.config.use_cache = True
    model.generation_config.max_length = 20
    rows = [json.loads(line) for line in args.input.read_text().splitlines() if line.strip()]
    def generate(text):
        model_input, bindings = bind(text) if manifest["data_manifest"].get("literal_binding") else (text, {})
        x = tokenizer(model_input, return_tensors="pt", truncation=False)
        if x.input_ids.shape[1] > 192: raise ValueError("input exceeds hard token limit")
        with torch.inference_mode():
            output = model.generate(**x, max_new_tokens=256, do_sample=False, num_beams=1)[0]
        if tokenizer.eos_token_id not in output.tolist(): raise ValueError("generation did not terminate")
        raw = tokenizer.decode(output, skip_special_tokens=True, clean_up_tokenization_spaces=False)
        if not manifest["data_manifest"].get("literal_binding"): return raw
        try: return encode(restore(decode(raw), bindings))
        except (ValueError, TypeError, KeyError): return "INVALID_MODEL_OUTPUT"
    for row in rows[:3]: generate(row["text"])
    predictions, latencies = [], []
    for row in rows:
        start = time.perf_counter()
        output = generate(row["text"])
        elapsed = 1000 * (time.perf_counter() - start)
        latencies.append(elapsed)
        predictions.append({**row, "prediction": output, "elapsed_ms": elapsed})
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output/"predictions.jsonl").write_text("".join(json.dumps(r,ensure_ascii=False)+"\n" for r in predictions))
    result = score(predictions)
    gpu_rows = [json.loads(line) for line in (args.artifacts/"challenge-predictions.jsonl").read_text().splitlines()]
    if manifest["data_manifest"].get("literal_binding"): gpu_rows = restore_predictions(gpu_rows)
    gpu = {r["id"]: r["parsed"] for r in gpu_rows}
    parity = []
    for row in predictions:
        try: parsed = decode(row["prediction"])
        except ValueError: parsed = None
        if parsed != gpu.get(row["id"]): parity.append(row["id"])
    result.update({"runtime": "PyTorch CPU, one thread, warm model; not native quantized inference", "cpu_gpu_semantic_difference_ids": parity,
                   "median_ms": statistics.median(latencies), "p95_ms": sorted(latencies)[int((len(latencies)-1)*.95)],
                   "model_fp32_bytes": sum(p.stat().st_size for p in model_dir.iterdir() if p.is_file())})
    (args.output/"summary.json").write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    print(json.dumps({k:v for k,v in result.items() if k != "errors"}, indent=2))


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--artifacts", type=Path, required=True)
    ap.add_argument("--input", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    main(ap.parse_args())
