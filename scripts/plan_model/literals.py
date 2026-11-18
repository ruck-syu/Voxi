"""Generic quoted-value binding, with no command or scheduling grammar.

The model sees stable ordinary-vocabulary aliases; original Unicode/bytes stay
outside generation. This is value integrity, not an action/role certificate.
"""
import json
import re


def bind(text):
    spans, i = [], 0
    pairs = {'"': '"', '“': '”', '‘': '’', "'": "'"}
    while i < len(text):
        opening = text[i]
        if opening not in pairs or opening == "'" and i > 0 and not text[i-1].isspace():
            i += 1
            continue
        start, i = i, i + 1
        while i < len(text):
            if text[i] == "\\" and opening == '"': i += 2; continue
            if text[i] == pairs[opening]: break
            i += 1
        if i == len(text): raise ValueError("unclosed quoted value")
        raw = text[start:i+1]
        value = json.loads(raw) if opening == '"' else raw[1:-1]
        spans.append((start, i+1, value))
        i += 1
    bindings = {}
    parts, end, number = [], 0, 0
    for start, stop, value in spans:
        existing = next((key for key, item in bindings.items() if item == value), None)
        if existing is not None:
            parts.extend([text[end:start], json.dumps(existing)])
            end = stop
            continue
        while f"~{number}~" in text: number += 1
        alias = f"~{number}~"
        number += 1
        parts.extend([text[end:start], json.dumps(alias)])
        bindings[alias] = value
        end = stop
    parts.append(text[end:])
    return "".join(parts), bindings


def substitute_target(value, bindings):
    if value == "REFUSE": return value
    reverse = {}
    for alias, literal in bindings.items(): reverse.setdefault(literal, alias)
    result = {"steps": [], "schedule": value["schedule"]}
    roles = {"mkdir": (1, 2), "move": (2, 3), "copy": (2, 3), "rename": (1, 2, 3), "trash": (2,)}
    for original in value["steps"]:
        step = list(original)
        for index in roles[step[0]]:
            if isinstance(step[index], str): step[index] = reverse.get(step[index], step[index])
        result["steps"].append(step)
    return result


def restore(value, bindings):
    if value == "REFUSE": return value
    used = set()
    def visit(item):
        if isinstance(item, str):
            # SentencePiece inserts spaces beside the checkpoint's added `~`
            # token. Normalize only a complete virtual alias, never user text.
            match = re.fullmatch(r"\s*~\s*(\d+)\s*~\s*", item)
            if match:
                alias = f"~{match[1]}~"
                if alias not in bindings: raise ValueError("invented literal alias")
                used.add(alias)
                return bindings[alias]
            if re.search(r"~\s*\d+\s*~", item): raise ValueError("embedded literal alias")
            return item
        if isinstance(item, list): return [visit(v) for v in item]
        if isinstance(item, dict): return {k:visit(v) for k,v in item.items()}
        return item
    result = visit(value)
    if used != set(bindings): raise ValueError("quoted source value omitted")
    return result
