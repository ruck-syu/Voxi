"""Matched small raw/slot fine-tunes on grouped development data only."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time

from parser_model_eval import digest, inventory, read_rows, rust_score, summarize, write_json, write_rows, fallback_eligible
from parser_slots import substitute


def prepare(experiment, output, train_limit, validation_limit, run):
    legacy = experiment / "legacy"
    current = {r['id']: r for r in read_rows(experiment / run / "evaluation.jsonl")}
    raw = read_rows(legacy / "gen/clean.jsonl")
    random.Random(19).shuffle(order := list(range(len(raw))))
    groups, selected, reasons = {"train": set(), "validation": set()}, {"train": [], "validation": []}, Counter()
    for index in order:
        original = raw[index]
        row = current[f"generated-development:{index}"]
        if not fallback_eligible(row):
            reasons['rules_accept'] += 1
            continue
        try:
            bundle = substitute(original['src'])
            target = bundle.encode_target(original['tgt'])
        except ValueError as error:
            reasons[str(error)] += 1
            continue
        # Group supported cases by canonical target, preventing exact target/seed
        # leakage across the train/validation split. REFUSE is grouped by text.
        group = original['tgt'] if original['tgt'] != 'REFUSE' else original['src']
        partition = 'validation' if int(hashlib.sha256(group.encode()).hexdigest()[:8], 16) % 5 == 0 else 'train'
        limit = validation_limit if partition == 'validation' else train_limit
        if len(selected[partition]) >= limit:
            continue
        groups[partition].add(group)
        selected[partition].append({**row, 'src': original['src'], 'tgt': original['tgt'],
                                    'slot_src': bundle.encoded, 'slot_tgt': target})
    assert not groups['train'] & groups['validation']
    for name, rows in selected.items():
        write_rows(output / f'{name}.jsonl', rows)
    write_json(output / 'data-manifest.json', {
        'source_sha256': digest(legacy / 'gen/clean.jsonl'),
        'evaluation_sha256': digest(experiment / run / 'evaluation.jsonl'),
        'train': len(selected['train']), 'validation': len(selected['validation']),
        'grouping': 'canonical target for supported commands; exact source text for REFUSE',
        'train_groups': len(groups['train']), 'validation_groups': len(groups['validation']),
        'selection_exclusions': dict(reasons),
        'train_sha256': digest(output / 'train.jsonl'), 'validation_sha256': digest(output / 'validation.jsonl'),
        'fresh_set_used': False, 'note': 'Inspected generated development data, not a pristine holdout. Selection is limited to representable currently-refused examples.'})


def train(base, rows, output, slots, epochs):
    import torch
    from transformers import AutoModelForSeq2SeqLM, AutoTokenizer
    torch.manual_seed(19)
    torch.set_num_threads(2)
    tok = AutoTokenizer.from_pretrained(base, local_files_only=True)
    model = AutoModelForSeq2SeqLM.from_pretrained(base, local_files_only=True).cpu().train()
    optimizer = torch.optim.AdamW(model.parameters(), lr=3e-4, weight_decay=.01)
    order = list(rows)
    randomizer = random.Random(19)
    losses, start = [], time.perf_counter()
    for epoch in range(epochs):
        randomizer.shuffle(order)
        for b in range(0, len(order), 32):
            batch = order[b:b+32]
            inputs = tok([r['slot_src' if slots else 'src'] for r in batch], max_length=64, truncation=True, padding=True, return_tensors='pt')
            labels = tok([r['slot_tgt' if slots else 'tgt'] for r in batch], max_length=64, truncation=True, padding=True, return_tensors='pt').input_ids
            labels[labels == tok.pad_token_id] = -100
            loss = model(**inputs, labels=labels).loss
            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.)
            optimizer.step()
            losses.append(float(loss.detach()))
        print(f"{'slots' if slots else 'raw'} epoch {epoch+1}/{epochs} loss {losses[-1]:.4f}", flush=True)
    model.eval().save_pretrained(output)
    tok.save_pretrained(output)
    return {'epochs': epochs, 'steps': len(losses), 'seconds': time.perf_counter() - start,
            'initial_loss': losses[0], 'final_loss': losses[-1], 'all_losses': losses}


def evaluate(experiment, output, suffix=''):
    import importlib.util
    legacy = experiment / 'legacy'
    spec = importlib.util.spec_from_file_location('frozen_guard', legacy / 'guard.py')
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    validation = read_rows(output / 'validation.jsonl')
    summary = {}
    base = legacy / 'out-v3/model_t5-efficient-tiny-v3'
    for name, model, extra in [('v3', base, []), ('raw-control', output/'model-raw', ['--semantic-guard']),
                               ('slots', output/'model-slots', ['--slots', '--semantic-guard'])]:
        run = output / (name + suffix)
        subprocess.run(['rtk', 'proxy', sys.executable, str(Path(__file__).with_name('parser_model_infer.py')),
                        '--model', str(model), '--input', str(output/'validation.jsonl'), '--output', str(run), *extra], check=True)
        predictions = read_rows(run/'predictions.jsonl')
        scored = rust_score([{**r, 'rewrite': p['rewrite']} for r, p in zip(validation, predictions)], run/'rust-score')
        for row in scored:
            row['combined'] = row['got'] if guard.safe(row['text'], row['rewrite']) else '!'
        write_rows(run/'outcomes.jsonl', scored)
        summary[name] = {'scores': summarize(scored, 'combined'),
                         'guard_rejections': sum(not guard.safe(r['text'], r['rewrite']) for r in scored),
                         'slot_rejections': sum(bool(p.get('slot_rejection')) for p in predictions),
                         'semantic_rejections': sum(bool(p.get('semantic_rejection')) for p in predictions),
                         'model_manifest_sha256': digest(run/'manifest.json')}
        if name == 'v3':
            from parser_slots import semantic_safe
            guarded = [dict(r, combined=r['combined'] if semantic_safe(r['text'],r['rewrite']) else '!') for r in scored]
            summary['v3-semantic-guard'] = {'scores': summarize(guarded,'combined')}
    write_json(output/f'summary{suffix}.json', summary)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--experiment', type=Path, required=True)
    p.add_argument('--output-name', default='slots-experiment-001')
    p.add_argument('--train-limit', type=int, default=1600)
    p.add_argument('--validation-limit', type=int, default=400)
    p.add_argument('--epochs', type=int, default=3)
    p.add_argument('--run', required=True, help='Current policy-aware rules refresh')
    args = p.parse_args()
    experiment = args.experiment.resolve()
    output = experiment / args.output_name
    output.mkdir(exist_ok=False)
    for file in ['parser_slot_experiment.py', 'parser_slots.py', 'parser_model_infer.py', 'parser_model_eval.py']:
        shutil.copy2(Path(__file__).parent/file, output/file)
    prepare(experiment, output, args.train_limit, args.validation_limit, args.run)
    rows = read_rows(output/'train.jsonl')
    base = experiment/'legacy/out-v3/model_t5-efficient-tiny-v3'
    training = {name: train(base, rows, output/f'model-{name}', name == 'slots', args.epochs) for name in ['raw','slots']}
    write_json(output/'training.json', training)
    evaluate(experiment, output)
    write_json(output/'artifacts.json', inventory(output))
    print(f'Completed controlled experiment: {output}', flush=True)


if __name__ == '__main__':
    main()
