"""Kaggle GPU fine-tune: v3 language rewrites -> typed multi-step plans.

Challenge labels are read only after the final checkpoint is saved. The job
does not touch the app or execute any generated action. No API keys required.
"""
from pathlib import Path
import hashlib
import importlib.metadata
import json
import math
import os
import random
import sys
import time
import zipfile

import torch
from transformers import AutoModelForSeq2SeqLM, AutoTokenizer


def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def read_rows(path): return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
def write_json(path, value): Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def main():
    if not torch.cuda.is_available(): raise RuntimeError("Kaggle GPU unavailable; refusing a silent CPU training run")
    candidates = []
    for path in Path("/kaggle/input").rglob("manifest.json"):
        try: value = json.loads(path.read_text())
        except (ValueError, OSError): continue
        if value.get("schema") == "voxi-plan-v2" and "base_files" in value: candidates.append(path.parent)
    if len(candidates) != 1: raise RuntimeError(f"expected one pinned training dataset, found {len(candidates)}")
    data = candidates[0]
    sys.path.insert(0, str(data))
    from schema import VERSION, encode, decode
    from literals import bind, restore, substitute_target
    out = Path("/kaggle/working")
    manifest = json.loads((data / "manifest.json").read_text())
    # Kaggle may expand uploaded ZIP resources in the mounted input. Accept
    # either layout, but load only the exact checkpoint hashes in the manifest.
    if (data / "base-v3.zip").is_file():
        base = Path("/tmp/voxi-base-v3")
        with zipfile.ZipFile(data / "base-v3.zip") as archive:
            if any(Path(name).is_absolute() or ".." in Path(name).parts for name in archive.namelist()):
                raise RuntimeError("unsafe base archive")
            archive.extractall(base)
    else:
        weights = [p for p in data.rglob("model.safetensors") if sha(p) == manifest["base_files"]["model.safetensors"]]
        if len(weights) != 1: raise RuntimeError(f"expected one hash-matched expanded checkpoint, found {len(weights)}")
        base = weights[0].parent
    for name, expected in manifest["base_files"].items():
        if sha(base / name) != expected: raise RuntimeError(f"base checkpoint hash mismatch: {name}")
    for split in ("train", "validation"):
        if sha(data / f"{split}.jsonl") != manifest["splits"][split]["sha256"]:
            raise RuntimeError(f"data hash mismatch: {split}")
    if sha(data / "challenge.jsonl") != manifest["challenge"]["sha256"]: raise RuntimeError("challenge hash mismatch")
    random.seed(417); torch.manual_seed(417); torch.cuda.manual_seed_all(417)
    torch.set_num_threads(2)
    torch.backends.cuda.matmul.allow_tf32 = True
    tokenizer = AutoTokenizer.from_pretrained(base, local_files_only=True)
    # Version 2 emits arrays using only the checkpoint's existing vocabulary.
    added = 0
    tokenizer.model_max_length = 192
    model = AutoModelForSeq2SeqLM.from_pretrained(base, local_files_only=True)
    model.config.use_cache = False
    model.to("cuda")
    train, validation = read_rows(data / "train.jsonl"), read_rows(data / "validation.jsonl")
    def tokenize(rows):
        encoded = []
        for row in rows:
            if manifest.get("literal_binding"):
                model_input, bindings = bind(row["text"])
                model_target = substitute_target(row["target"], bindings)
                if (model_input, bindings, model_target) != (row["model_input"], row["bindings"], row["model_target"]):
                    raise RuntimeError("literal preprocessing mismatch")
            else:
                model_input, model_target = row["text"], row["target"]
            ids = tokenizer(model_input, truncation=False)["input_ids"]
            target = encode(model_target)
            labels = tokenizer(text_target=target, truncation=False)["input_ids"]
            if labels[0] == tokenizer.unk_token_id: raise RuntimeError("unknown first target token")
            if len(ids) > 192 or len(labels) > 256:
                raise RuntimeError(f"refusing truncation: {row['id']} input={len(ids)} output={len(labels)}")
            encoded.append((ids, labels))
        return encoded
    train_tokens, validation_tokens = tokenize(train), tokenize(validation)
    def collate(rows):
        xs, ys = zip(*rows)
        x = tokenizer.pad({"input_ids": list(xs)}, padding=True, return_tensors="pt").to("cuda")
        labels = torch.full((len(ys), max(map(len, ys))), -100, dtype=torch.long, device="cuda")
        for i, value in enumerate(ys): labels[i, :len(value)] = torch.tensor(value, device="cuda")
        return x, labels
    batch_size, epochs = 64, 8
    steps = epochs * math.ceil(len(train) / batch_size)
    optimizer = torch.optim.AdamW(model.parameters(), lr=5e-4, weight_decay=.01)
    def lr_factor(step):
        warmup = max(1, int(steps * .04))
        return (step + 1) / warmup if step < warmup else max(.05, (steps - step) / max(1, steps - warmup))
    scheduler = torch.optim.lr_scheduler.LambdaLR(optimizer, lr_factor)
    start, history, best = time.perf_counter(), [], float("inf")
    output_model = out / "model-voxi-plan-v4"
    order = list(range(len(train_tokens)))
    print(json.dumps({"gpu": torch.cuda.get_device_name(0), "train": len(train), "validation": len(validation), "parameters": sum(p.numel() for p in model.parameters()), "steps": steps, "added_tokens": added}), flush=True)
    global_step = 0
    for epoch in range(epochs):
        random.shuffle(order)
        model.train()
        total = 0.
        for offset in range(0, len(order), batch_size):
            x, labels = collate([train_tokens[i] for i in order[offset:offset+batch_size]])
            optimizer.zero_grad(set_to_none=True)
            loss = model(**x, labels=labels).loss
            if not torch.isfinite(loss): raise RuntimeError("non-finite loss")
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.)
            optimizer.step(); scheduler.step()
            total += loss.item() * len(labels)
            global_step += 1
            if global_step % 50 == 0:
                print(f"epoch={epoch+1} step={global_step}/{steps} loss={loss.item():.5f} elapsed={time.perf_counter()-start:.1f}s", flush=True)
        model.eval()
        val = 0.
        with torch.inference_mode():
            for offset in range(0, len(validation_tokens), batch_size):
                batch = validation_tokens[offset:offset+batch_size]
                x, labels = collate(batch)
                val += model(**x, labels=labels).loss.item() * len(batch)
        record = {"epoch": epoch+1, "train_loss": total/len(train), "validation_loss": val/len(validation), "elapsed_seconds": time.perf_counter()-start}
        history.append(record)
        print(json.dumps(record), flush=True)
        if record["validation_loss"] < best:
            best = record["validation_loss"]
            model.save_pretrained(output_model)
            tokenizer.save_pretrained(output_model)
            write_json(out / "selected-checkpoint.json", record)
        write_json(out / "training-history.json", history)
    del model, optimizer
    torch.cuda.empty_cache()
    model = AutoModelForSeq2SeqLM.from_pretrained(output_model, local_files_only=True).to("cuda").eval()
    model.config.use_cache = True
    model.generation_config.max_length = 256
    def predict(rows, name):
        predictions = []
        for offset in range(0, len(rows), 32):
            batch = rows[offset:offset+32]
            inputs = [bind(r["text"])[0] if manifest.get("literal_binding") else r["text"] for r in batch]
            x = tokenizer(inputs, padding=True, truncation=False, return_tensors="pt").to("cuda")
            if x.input_ids.shape[1] > 192: raise RuntimeError("evaluation input exceeds limit")
            with torch.inference_mode():
                generated = model.generate(**x, max_new_tokens=256, do_sample=False, num_beams=1)
            texts = tokenizer.batch_decode(generated, skip_special_tokens=True, clean_up_tokenization_spaces=False)
            for row, text, token_ids in zip(batch, texts, generated):
                try:
                    if tokenizer.eos_token_id not in token_ids.tolist(): raise ValueError("generation did not terminate")
                    parsed = decode(text)
                    if manifest.get("literal_binding"):
                        parsed = restore(parsed, bind(row["text"])[1])
                    normalized, error = encode(parsed), None
                except (ValueError, TypeError, KeyError) as exc:
                    parsed, normalized, error = None, "INVALID_MODEL_OUTPUT", str(exc)
                predictions.append({**row, "raw_prediction": text, "prediction": normalized, "parsed": parsed, "schema_error": error, "exact": parsed == row["target"]})
        (out / f"{name}-predictions.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in predictions))
        refusals = [r for r in predictions if r["target"] == "REFUSE"]
        return {"rows": len(rows), "exact": sum(r["exact"] for r in predictions), "invalid_schema": sum(r["schema_error"] is not None for r in predictions), "refusal_rows": len(refusals), "accepted_refusals": [r["id"] for r in refusals if isinstance(r["parsed"], dict)], "wrong_accepted_plans": [r["id"] for r in predictions if isinstance(r["parsed"], dict) and not r["exact"]]}
    validation_score = predict(validation, "validation")
    # First access to the frozen challenge labels; never used for selection.
    challenge_score = predict(read_rows(data / "challenge.jsonl"), "challenge")
    model.save_pretrained(output_model)
    write_json(out / "run-manifest.json", {"schema": VERSION, "device": torch.cuda.get_device_name(0), "base_hashes": manifest["base_files"], "data_manifest": manifest, "script_sha256": sha(__file__), "schema_sha256": sha(data/"schema.py"), "versions": {p: importlib.metadata.version(p) for p in ("torch", "transformers", "tokenizers", "safetensors")}, "epochs": epochs, "batch_size": batch_size, "steps": global_step, "training_seconds": time.perf_counter()-start, "selected_checkpoint": json.loads((out/"selected-checkpoint.json").read_text()), "model_files": {p.name: sha(p) for p in sorted(output_model.iterdir()) if p.is_file()}, "max_input_tokens": 192, "max_new_tokens": 256, "validation": validation_score, "challenge": challenge_score, "integration_authorized": False, "note": "Schema validation alone is not a semantic certificate. Native export, local latency, and planner integration require separate evaluation."})
    print("RESULT " + json.dumps({"validation": validation_score, "challenge": challenge_score}), flush=True)


if __name__ == "__main__": main()
