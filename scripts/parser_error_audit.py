"""Classify generated parser diagnostics without changing inputs or labels.

Usage: python3 scripts/parser_error_audit.py --input-dir <feas/gen> --output <prefix>
Outputs <prefix>.jsonl and <prefix>.md. Review categories are triage hints,
not corrected labels or evidence that the planner permits an operation.
"""
import argparse
from collections import Counter
from datetime import date
import hashlib
import json
from pathlib import Path
import re


def flatten(value, prefix=""):
    if isinstance(value, dict):
        result = {}
        for key, child in value.items():
            result.update(flatten(child, f"{prefix}.{key}" if prefix else key))
        return result
    return {prefix: value}


def classify(row, kind):
    if kind == "paraphrase":
        actual, expected = flatten(row["rules"]), flatten(row["want"])
        fields = sorted(k for k in actual.keys() | expected.keys() if actual.get(k) != expected.get(k))
        if fields == ["src.filter.before"]:
            try:
                delta = (date.fromisoformat(actual.get(fields[0])) - date.fromisoformat(expected.get(fields[0]))).days
            except (ValueError, TypeError):
                delta = 0
            if delta == 1 and re.search(r"\b(between|through|until|to)\b", row["text"], re.I):
                return "review_date_endpoint_label", fields
        if fields == ["src.folder"]:
            folder = expected.get("src.folder")
            if isinstance(folder, str) and folder.lower() not in row["text"].lower():
                return "review_unstated_source_folder", fields
        if "src.kind" in fields:
            return "source_item_or_set", fields
        if any(k.endswith("folder") or k == "parent" for k in fields):
            return "source_or_folder_grammar", fields
        if "to" in fields or "output" in fields or "names" in fields:
            return "destination_output_or_name_grammar", fields
        if "intent" in fields:
            return "command_selection", fields
        return "filter_or_value_grammar", fields
    text = row["text"]
    if re.search(r"(?:~/|/)(?:Library|System|Applications|usr|etc|private|bin|opt|Users/Shared)\b|\b(?:library|system|applications|keychains)\b|(?<!\w)\.(?:ssh|config|env|git|plist|bash_history|DS_Store|localized)\b|\bhidden\b|\w+\.app\b", text, re.I):
        return "review_planner_scope_or_hidden_path", []
    if re.search(r"\b(?:dropbox|icloud|onedrive|google\s+(?:drive|photos)|backblaze|auntie|supervisor|whatsapp|wifi|bluetooth|airdrop|mum|slack|client|colleague|phone|ipad|printer|hiring|legal|committee|cousin|accountant|cloud)\b", text, re.I) or "Priya" in text:
        return "review_recipient_or_cloud_vs_local_folder", []
    if re.search(r"\bcompress\b", text, re.I):
        return "review_archive_vs_image_compression_intent", []
    if re.search(r"\b(?:from|dated|days|hours|week|year|birthday|future|season|autumn|month)\b|20\d{2}", text, re.I):
        return "unsupported_or_ambiguous_time_filter", []
    if re.search(r"\b(?:excluding|bar|apart|matching|like)\b|\[[^]]+\]", text, re.I):
        return "unsupported_filter_syntax", []
    return "unsupported_or_ambiguous_request", []


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    records, hashes = [], {}
    for kind, name in [("paraphrase", "para-misreads.json"), ("refusal", "refusals-accepted.json")]:
        path = args.input_dir / name
        raw = path.read_bytes()
        hashes[name] = hashlib.sha256(raw).hexdigest()
        for row in json.loads(raw):
            category, fields = classify(row, kind)
            records.append({"kind": kind, "category": category, "differing_fields": fields, **row})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.with_suffix(".jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records))
    lines = ["# Generated parser error triage", "", "Categories are review hints. Inputs and expected labels are unchanged.", "No category proves an operation can execute; scope checks belong to the planner.", ""]
    for kind in ["paraphrase", "refusal"]:
        counts = Counter(r["category"] for r in records if r["kind"] == kind)
        lines.extend([f"## {kind.title()} ({sum(counts.values())})", ""])
        lines.extend(f"- {name}: {count}" for name, count in counts.most_common())
        lines.append("")
    lines.extend(["## Source hashes (SHA-256)", ""])
    lines.extend(f"- `{name}`: `{digest}`" for name, digest in hashes.items())
    args.output.with_suffix(".md").write_text("\n".join(lines) + "\n")
    print(f"Wrote {len(records)} triaged records to {args.output.with_suffix('.jsonl')}")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
