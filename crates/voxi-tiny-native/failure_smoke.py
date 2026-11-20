#!/usr/bin/env python3
"""Reproduce native input, no-EOS, missing-model, and corrupt-model checks."""
import argparse, json, shutil, subprocess, tempfile
from pathlib import Path

def main():
    p=argparse.ArgumentParser()
    p.add_argument("--binary",type=Path,required=True)
    p.add_argument("--model-dir",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    a=p.parse_args()
    a.output.parent.mkdir(parents=True,exist_ok=True)
    rows=[{"id":"empty","text":""},{"id":"wrong-type","text":12},
          {"id":"too-long","text":" ".join(f"token{i}" for i in range(200))},
          {"id":"no-eos","text":" ".join(f"token{i}" for i in range(30))}]
    proc=subprocess.run([str(a.binary),"--model-dir",str(a.model_dir),"--threads","1"],
        input="{bad json}\n"+"".join(json.dumps(r)+"\n" for r in rows),text=True,capture_output=True)
    with tempfile.TemporaryDirectory(prefix="voxi-model-failure-") as temp:
        missing=Path(temp)/"missing"
        corrupt=Path(temp)/"corrupt"
        corrupt.mkdir()
        shutil.copy2(a.model_dir/"config.json",corrupt/"config.json")
        shutil.copy2(a.model_dir/"tokenizer.json",corrupt/"tokenizer.json")
        (corrupt/"model-q8_0.gguf").write_bytes(b"not a gguf")
        mismatch=Path(temp)/"valid-config-mismatch"
        mismatch.mkdir()
        for name in ("config.json","tokenizer.json","model-q8_0.gguf"):
            shutil.copy2(a.model_dir/name,mismatch/name)
        config=json.loads((mismatch/"config.json").read_text())
        config["decoder_start_token_id"]=1
        (mismatch/"config.json").write_text(json.dumps(config))
        bad=[]
        for model in (missing,corrupt,mismatch):
            res=subprocess.run([str(a.binary),"--model-dir",str(model),"--threads","1"],text=True,capture_output=True)
            bad.append({"model_dir_kind":model.name,"exit_code":res.returncode,"stderr":res.stderr.strip()})
    result={"request_process_exit_code":proc.returncode,"load_stderr":proc.stderr.strip(),
        "per_line_results":[json.loads(line) for line in proc.stdout.splitlines() if line.strip()],
        "load_failures":bad}
    assert proc.returncode==0, proc.stderr
    assert len(result["per_line_results"])==5
    assert all(row.get("rewrite") is None and row.get("error") for row in result["per_line_results"])
    assert "input_too_long" in result["per_line_results"][3]["error"]
    assert "without eos" in result["per_line_results"][4]["error"]
    assert all(row["exit_code"]!=0 for row in bad)
    assert "package_mismatch" in bad[-1]["stderr"], "valid config mismatch must fail before inference"
    a.output.write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n")
    print(json.dumps({"request_process_exit_code":proc.returncode,
       "per_line_errors":[r.get("error") for r in result["per_line_results"]],"load_failures":bad},indent=2))
if __name__=="__main__": main()
