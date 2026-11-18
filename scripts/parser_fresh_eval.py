"""Evaluate frozen independent labels once, without tuning on resulting failures."""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys

from parser_model_eval import (CRATE, ROOT, READ_ONLY, category, digest, inventory,
                               provenance, read_rows, rust_score, summarize,
                               validate_predictions, write_json, write_rows, fallback_eligible, rule_decision)


def finish_row(original, scored, prediction, safe):
    eligible = fallback_eligible(original)
    row = dict(original, combined=original['rules'], decision=rule_decision(original))
    if not eligible:
        return row
    row.update({k: prediction[k] for k in ('rewrite', 'raw_rewrite', 'model_input', 'slot_rejection', 'semantic_rejection') if k in prediction})
    row['rewrite_outcome'] = scored['got']
    row['rewrite_parse_status'] = scored.get('parse_status')
    row['rewrite_hard_refusal_reason'] = scored.get('hard_refusal_reason')
    allowed = safe(row['text'], row['rewrite'])
    schedule_same = (original.get('schedule_due'), original.get('schedule_rule')) == (scored.get('schedule_due'), scored.get('schedule_rule'))
    row['guard_allowed'] = allowed
    row['schedule_guard_allowed'] = schedule_same
    if not allowed:
        row['decision'] = 'guard_rejection'
    elif not schedule_same and scored['got'] != '!':
        row['decision'] = 'schedule_guard_rejection'
    else:
        row['combined'] = scored['got']
        row['decision'] = 'model_refusal' if scored['got'] == '!' else 'model_accepted'
        return row
    row['combined'] = '!'
    return row


def verify_inference(folder, input_path, model_files):
    meta = json.loads((folder/'manifest.json').read_text())
    checks = [digest(folder/'predictions.jsonl') == meta['predictions_sha256'],
              digest(input_path) == meta['input_sha256'],
              digest(folder/'inference-script.py') == meta['script_sha256'],
              meta['model_files'] == model_files == inventory(Path(meta['model']))]
    for name, expected in meta['helper_hashes'].items():
        checks.append(digest(folder/name) == expected)
    if not all(checks):
        raise ValueError('Inference provenance mismatch')
    return meta


def gates(rows, inference):
    errors = [r for r in rows if r['combined'] != '!' and r['combined'] != r['want']]
    violations = sum(r['want'] == '!' for r in errors)
    mutating = sum(r['want'] != '!' and r['combined']['intent'] not in READ_ONLY for r in errors)
    eligible_supported = [r for r in rows if r['want'] != '!' and fallback_eligible(r)]
    recovered = sum(r['combined'] == r['want'] for r in eligible_supported)
    coverage = recovered / len(eligible_supported) if eligible_supported else 0
    p95 = inference['single_sentence_benchmark']['p95_ms']
    passed = {'no_refuse_violations': violations == 0, 'no_file_changing_misreads': mutating == 0,
              'supported_recovery_at_least_10_percent': coverage >= .1,
              'local_cpu_p95_at_most_200_ms': p95 is not None and p95 <= 200,
              'model_and_tokenizer_at_most_25_mb': inference['model_bytes'] <= 25_000_000,
              'accepted_rules_unchanged': all(r['combined'] == r['rules'] for r in rows if not fallback_eligible(r)),
              'native_app_runtime_validated': False}
    return {'checks': passed, 'qualifies_for_integration': all(passed.values()),
            'refuse_violations': violations, 'file_changing_misreads': mutating,
            'supported_fallbacks': len(eligible_supported), 'supported_recovered': recovered,
            'recovery_fraction': coverage, 'note': 'Native runtime, shipped dependencies, previews, confirmation and planner restrictions require validation before any app integration.'}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--experiment', type=Path, required=True)
    p.add_argument('--output-name', default='fresh-evaluation-001')
    p.add_argument('--dataset', type=Path, default=ROOT/'docs/testing/parser-model-fresh-2026-11-15.jsonl')
    args = p.parse_args()
    experiment = args.experiment.resolve()
    source = args.dataset.resolve()
    source_manifest = source.with_suffix('.manifest.json')
    frozen = json.loads(source_manifest.read_text())
    if digest(source) != frozen['sha256']:
        raise ValueError('Frozen independent labels changed')
    output = experiment/args.output_name
    output.mkdir(exist_ok=False)
    meta = provenance(output/'provenance')
    shutil.copy2(source,output/'frozen-labels.jsonl')
    shutil.copy2(source_manifest,output/'frozen-labels.manifest.json')
    original = read_rows(source)
    expected_count = frozen.get('case_count', frozen.get('count'))
    if not len(original) == expected_count == len({r['id'] for r in original}):
        raise ValueError('Frozen dataset count or IDs changed')
    rows = [{**r,'set':'fresh-independent','split':'independent-agent-authored',
             'at':r['id'],'label_source_sha256':digest(source),'rewrite':r['text']} for r in original]
    source_inventory = inventory(CRATE/'src')
    scored = rust_score(rows,output/'rules-score')
    invalid = [r['id'] for r in scored if r['want'] != '!' and not r.get('want_schema_valid',False)]
    if invalid:
        raise ValueError(f'Frozen labels are not canonical Intent schema; leave labels unchanged and report invalid IDs: {invalid}')
    rows = [{**r,'rules':r['got']} for r in scored]
    for row in rows:
        fallback_eligible(row)
    write_rows(output/'evaluation.jsonl',rows)
    fallback = [r for r in rows if fallback_eligible(r)]
    write_rows(output/'fallback.jsonl',fallback)
    spec = importlib.util.spec_from_file_location('frozen_guard',experiment/'legacy/guard.py')
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    summary = {'rules':summarize(rows,'rules'),'rules_decisions':dict(Counter(r['parse_status'] for r in rows)), 'candidates':{}}
    base = experiment/'legacy/out-v3/model_t5-efficient-tiny-v3'
    models = [('v3',base,[]),('v3-shared-guard',base,['--semantic-guard']),
              ('raw-control',experiment/'slots-experiment-001/model-raw',['--semantic-guard']),
              ('slots',experiment/'slots-experiment-001/model-slots',['--slots','--semantic-guard'])]
    for name,model,extra in models:
        folder = output/name
        model_files = inventory(model)
        subprocess.run(['rtk','proxy',sys.executable,str(ROOT/'scripts/parser_model_infer.py'),
                        '--model',str(model),'--input',str(output/'fallback.jsonl'),
                        '--output',str(folder),*extra],check=True)
        inference = verify_inference(folder,output/'fallback.jsonl',model_files)
        predictions = validate_predictions(rows,read_rows(folder/'predictions.jsonl'))
        rewrites = rust_score([{**r,'rewrite':predictions[r['id']]['rewrite']} for r in fallback],folder/'rust-score')
        by_id = {r['id']:r for r in rewrites}
        combined = [finish_row(r,by_id.get(r['id']),predictions.get(r['id']),guard.safe) for r in rows]
        errors = [r for r in combined if r['combined']!='!' and r['combined']!=r['want']]
        write_rows(folder/'outcomes.jsonl',combined)
        write_rows(folder/'accepted-errors.jsonl',errors)
        summary['candidates'][name] = {'scores':summarize(combined,'combined'),
            'decisions':dict(Counter(r['decision'] for r in combined)),
            'slot_rejections':sum(bool(p.get('slot_rejection')) for p in predictions.values()),
            'semantic_rejections':sum(bool(p.get('semantic_rejection')) for p in predictions.values()),
            'inference':inference,'gates':gates(combined,inference)}
    if source_inventory != inventory(CRATE/'src') or digest(source) != frozen['sha256']:
        raise ValueError('Parser or fresh labels changed during evaluation')
    meta.update({'frozen_set':frozen,'frozen_sha256':digest(source),'source_inventory':source_inventory,
                 'guard_sha256':digest(experiment/'legacy/guard.py'),'summary_sha256':None,
                 'decision':'No candidate qualifies; integration is prohibited unless every gate passes.'})
    write_json(output/'summary.json',summary)
    meta['summary_sha256'] = digest(output/'summary.json')
    write_json(output/'manifest.json',meta)
    write_json(output/'artifacts.json',inventory(output))
    print(f'Frozen evaluation complete: {output}',flush=True)


if __name__ == '__main__':
    main()
