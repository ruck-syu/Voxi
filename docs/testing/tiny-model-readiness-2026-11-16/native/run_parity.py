#!/usr/bin/env python3
"""Run documented development parity/latency protocol against saved FP32 rows."""
import argparse, json, math, statistics, subprocess, time
from pathlib import Path

def read(path):
    with path.open() as f:
        return [json.loads(line) for line in f if line.strip()]

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--reference", type=Path, required=True)
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--model-dir", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--threads", type=int, default=1)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    rows = [r for r in read(a.reference) if str(r.get("id", "")).startswith("generated-development:")]
    if len(rows) < 300: raise SystemExit(f"need 300 generated-development rows, found {len(rows)}")
    n = len(rows)
    selected = [rows[i * n // 300] for i in range(300)]
    timed = [selected[i * 300 // 40] for i in range(40)]
    requested = timed[:3] + selected
    inp = a.output / "native-request.jsonl"
    with inp.open("w") as f:
        for row in requested: f.write(json.dumps({"id":row["id"],"text":row["text"]}, ensure_ascii=False)+"\n")
    start=time.perf_counter()
    proc=subprocess.run([str(a.binary),"--model-dir",str(a.model_dir),"--threads",str(a.threads)], input=inp.read_text(), text=True, capture_output=True)
    wall_ms=(time.perf_counter()-start)*1000
    if proc.returncode: raise SystemExit(f"native process failed ({proc.returncode}): {proc.stderr}")
    results=[json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
    if len(results) != len(requested) or any((got.get("id"), got.get("text")) != (want["id"], want["text"]) for got,want in zip(results,requested)):
        raise SystemExit("native output IDs/text are missing, duplicated, or out of order")
    warmups, preds=results[:3],results[3:]
    if len(preds)!=300: raise SystemExit(f"expected 300 predictions, got {len(preds)}")
    with (a.output/"predictions-native.jsonl").open("w") as f:
        for pred in preds: f.write(json.dumps(pred,ensure_ascii=False)+"\n")
    ref={r["id"]:r["rewrite"] for r in selected}
    mismatches=[]
    for pred in preds:
        expected=ref[pred["id"]]
        actual=pred.get("rewrite")
        if expected != actual:
            mismatches.append({"id":pred["id"],"text":pred.get("text"),"python_fp32":expected,"native_q8":actual,"error":pred.get("error")})
    latencies=[preds[i*300//40]["elapsed_ms"] for i in range(40)]
    sorted_ms=sorted(latencies)
    p95=sorted_ms[math.floor((len(sorted_ms)-1)*.95)]
    report={"reference_kind":"existing v3-local-fp32 generated-development outputs; parity oracle only",
      "reference_path":str(a.reference),"rows_in_development_subset":n,"selected_rows":len(selected),
      "warmups":len(warmups),"timed_samples":len(latencies),"thread_count":a.threads,
      "native_load_ms_log":proc.stderr.strip(),"process_wall_ms_including_one_load_and_303_generations":wall_ms,
      "latency_ms":{"p50":statistics.median(sorted_ms),"p95_index_floor":p95,"all_40":latencies},
      "exact_rewrite_matches":len(selected)-len(mismatches),"different_or_error":len(mismatches),"mismatches":mismatches}
    (a.output/"parity-latency.json").write_text(json.dumps(report,indent=2,ensure_ascii=False)+"\n")
    (a.output/"mismatches.json").write_text(json.dumps(mismatches,indent=2,ensure_ascii=False)+"\n")
    print(json.dumps({k:v for k,v in report.items() if k not in ("mismatches","latency_ms")},indent=2))

if __name__=="__main__": main()
