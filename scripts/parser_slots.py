"""Lossless typed slots for a bounded fallback experiment, not app integration.

Extraction sees only the user's sentence. Training targets may use only slots
already extracted from that sentence. Unsupported/ambiguous extraction is not
repaired using expected intents. Every original spelling and quote is retained.
"""
from collections import Counter
from dataclasses import dataclass
from decimal import Decimal
import re

MARKER = re.compile(r"\b(?:NAME|FOLDER|NUM)\d+\b")
MARKER_LIKE = re.compile(r"\b(?:NAME|FOLDER|NUM)\s*\d+\b", re.I)
QUANTITY = re.compile(r"(?<![\w.])\d[\d,]*(?:\.\d+)?(?:\s*(?:KB|MB|GB|TB|px|pixels?|days?|weeks?|months?|years?|hours?|minutes?))?\b", re.I)
TOKEN = re.compile(r"[^\s\"“”]+")
KNOWN_FOLDER = re.compile(r"\b(?:Desktop|Downloads|Documents|Pictures|Movies|Music|Archive|Backup|Clients|Invoices|Receipts|Screenshots)\b", re.I)
SIZE = {"kb": Decimal(1) / 1024, "mb": Decimal(1), "gb": Decimal(1024), "tb": Decimal(1024 ** 2)}
TIME = {"day": 1, "week": 7, "month": 30, "year": 365, "hour": Decimal(1) / 24, "minute": Decimal(1) / 1440}


@dataclass(frozen=True)
class Span:
    start: int
    end: int
    kind: str
    raw: str
    identity: tuple


def quantity_identity(raw):
    m = re.fullmatch(r"([\d,.]+)\s*([a-z]*)", raw, re.I)
    number, unit = Decimal(m[1].replace(",", "")), m[2].lower().rstrip("s")
    if unit in SIZE:
        return ("size", number * SIZE[unit])
    if unit in TIME:
        return ("duration", number * Decimal(TIME[unit]))
    return ("pixels" if unit in ("px", "pixel") else "number", number)


def quoted_spans(text):
    """Single quotes inside words are apostrophes, not delimiters."""
    i = 0
    while i < len(text):
        q = text[i]
        if q not in "\"'" or (q == "'" and i and text[i - 1].isalnum()):
            i += 1
            continue
        start = i
        i += 1
        while i < len(text):
            if text[i] == q and not (q == "'" and i + 1 < len(text) and text[i + 1].isalnum()):
                yield start, i + 1
                i += 1
                break
            i += 1
        else:
            raise ValueError("Unclosed quoted value")


def unquoted(text):
    chars = list(text)
    for start, end in quoted_spans(text):
        chars[start:end] = " " * (end - start)
    return "".join(chars)


def folder_context(text, start):
    prefix = text[:start].lower()
    if re.search(r"\b(?:in|from|inside|within|under|folder(?:\s+(?:named|called))?)\s+(?:(?:the|my)\s+)?$", prefix):
        return True
    return bool(re.search(r"\b(?:to|into)\s+$", prefix) and
                re.search(r"\b(?:move|copy|send|shift|transfer|relocate|unzip|unpack)\b", unquoted(prefix)))


def spans(text):
    if MARKER_LIKE.search(text):
        raise ValueError("Input collides with reserved slot syntax")
    found = []

    def add(start, end, kind, identity):
        if any(start < s.end and end > s.start for s in found):
            return
        found.append(Span(start, end, kind, text[start:end], identity))

    for start, end in quoted_spans(text):
        kind = "FOLDER" if folder_context(text, start) else "NAME"
        add(start, end, kind, (kind, text[start + 1:end - 1]))
    for m in TOKEN.finditer(text):
        raw = m[0].rstrip(",;?!")
        # Decimals are quantities. Dotted filenames and explicit paths are values.
        if ("/" in raw or re.search(r"\.[^\W\d_][\w-]*$", raw, re.UNICODE)) and not re.fullmatch(r"[\d.]+", raw):
            kind = "FOLDER" if folder_context(text, m.start()) else "NAME"
            add(m.start(), m.start() + len(raw), kind, (kind, raw))
    for m in KNOWN_FOLDER.finditer(text):
        # Common nouns like "documents" are file types unless in a folder context
        # or capitalized, avoiding forced slots for every lower-case type word.
        if folder_context(text, m.start()) or m[0][0].isupper():
            add(m.start(), m.end(), "FOLDER", ("FOLDER", m[0]))
    for m in QUANTITY.finditer(text):
        add(m.start(), m.end(), "NUM", ("NUM", *quantity_identity(m[0])))
    return sorted(found, key=lambda s: s.start)


@dataclass
class Bundle:
    original: str
    encoded: str
    slots: dict
    identities: dict
    counts: Counter

    def restore(self, rewrite):
        if rewrite.strip().upper() == "REFUSE":
            return "REFUSE"
        markers = MARKER.findall(rewrite)
        if Counter(markers) != self.counts or len(MARKER_LIKE.findall(rewrite)) != len(markers):
            raise ValueError("Missing, duplicated, invented, or malformed slots")
        masked = MARKER.sub(lambda m: " " * len(m[0]), rewrite)
        if spans(masked):
            raise ValueError("Unslotted protected values were invented or duplicated")
        family = action_family(self.original)
        if slot_roles(self.encoded, family) != slot_roles(rewrite, family):
            raise ValueError("Incompatible slot roles or source/destination direction")
        # A whole quantity slot already carries its units; a generated second unit
        # would be an incompatible type/value substitution.
        for marker in markers:
            if marker.startswith("NUM") and re.search(r"[a-z]", self.slots[marker], re.I):
                if re.search(rf"\b{marker}\s+(?:KB|MB|GB|TB|px|pixels?|days?|weeks?|months?|years?)\b", rewrite, re.I):
                    raise ValueError("A quantity slot cannot acquire a second unit")
        if action_family(self.original) == "replace":
            before = [m for m in MARKER.findall(self.encoded) if m.startswith("NAME")]
            after = [m for m in markers if m.startswith("NAME")]
            if before != after:
                raise ValueError("Rename replacement direction changed")
        return MARKER.sub(lambda m: self.slots[m[0]], rewrite)

    def encode_target(self, target):
        if target == "REFUSE":
            return target
        target_spans = spans(target)
        result, end = [], 0
        for span in target_spans:
            marker = self.identities.get(span.identity)
            if marker is None:
                raise ValueError("Target contains an untyped/incompatible value")
            result.extend([target[end:span.start], marker])
            end = span.end
        result.append(target[end:])
        encoded = "".join(result)
        self.restore(encoded)  # Exact multiplicities and direction must also hold.
        return encoded


def substitute(text):
    identities, slots, numbers, chunks = {}, {}, Counter(), []
    end = 0
    for span in spans(text):
        marker = identities.get(span.identity)
        if marker is None:
            numbers[span.kind] += 1
            marker = f"{span.kind}{numbers[span.kind]}"
            identities[span.identity] = marker
            slots[marker] = span.raw
        # Equal semantic numbers with different spellings must remain separate
        # source values, rather than losing exact bytes during a round trip.
        elif slots[marker] != span.raw:
            numbers[span.kind] += 1
            marker = f"{span.kind}{numbers[span.kind]}"
            slots[marker] = span.raw
            identities[(*span.identity, span.raw)] = marker
        chunks.extend([text[end:span.start], marker])
        end = span.end
    chunks.append(text[end:])
    encoded = "".join(chunks)
    bundle = Bundle(text, encoded, slots, identities, Counter(MARKER.findall(encoded)))
    if bundle.restore(encoded) != text:
        raise ValueError("Substitution failed exact round trip")
    return bundle


FAMILIES = {
    "move": ("move", "relocate", "shift", "transfer", "send", "drag", "drop", "put"),
    "copy": ("copy", "duplicate", "clone", "replicate"),
    "trash": ("delete", "trash", "remove", "erase", "bin", "discard"),
    "list": ("list", "show", "display", "find"), "count": ("count", "tally"),
    "zip": ("zip", "compress", "archive"), "unzip": ("unzip", "unpack", "decompress", "expand"),
    "rename": ("rename",), "replace": ("replace",), "resize": ("resize", "shrink", "scale"),
    "convert": ("convert",), "merge": ("merge", "combine", "join"), "extract": ("extract",),
    "split": ("split",), "create": ("create", "make"), "open": ("open", "reveal"),
    "lowercase": ("lowercase",), "sort": ("sort", "organize", "arrange"),
}


def action_family(text):
    text = unquoted(text).lower()
    if re.search(r"\bhow\s+many\b", text):
        return "count"
    # Names, paths, quoted content and words embedded in filenames aren't verbs.
    for word in re.findall(r"(?<![\w./])([a-z]+)(?![\w./])", text):
        families = [family for family, words in FAMILIES.items() if word in words]
        if families:
            return families[0]
    return None


def slot_roles(text, family):
    roles = Counter()
    for match in MARKER.finditer(text):
        marker = match[0]
        prefix = text[:match.start()].lower()
        target = bool(re.search(r"\b(?:to|into|as|with)\s+$", prefix))
        scope = bool(re.search(r"\b(?:in|from|inside|within|under)\s+$", prefix))
        if marker.startswith("NUM"):
            m = re.search(r"\b(over|under|larger than|bigger than|smaller than|less than|more than|older than|newer than)\s+$", prefix)
            role = ("lt" if m[1] in ("under", "smaller than", "less than", "newer than") else "gt") if m else "quantity"
            if family == "extract":
                if re.search(r"\bpages?\s+$", prefix):
                    role = "range_start"
                elif re.search(r"(?:\b(?:to|through|until)|-)\s*$", prefix):
                    role = "range_end"
        elif target:
            role = "destination" if family in ("move", "copy", "unzip") else "replacement" if family == "replace" else "output"
        elif scope or marker.startswith("FOLDER"):
            role = "source_scope"
        else:
            role = "source"
        roles[(marker, role)] += 1
    return roles


def constraints(text):
    text = unquoted(text).lower()
    found = set()
    for m in re.finditer(r"\b(over|under|larger than|bigger than|smaller than|less than|more than|older than|newer than)\s+(\d[\d,]*(?:\.\d+)?\s*(?:kb|mb|gb|tb|days?|weeks?|months?|years?))\b", text):
        direction = "lt" if m[1] in ("under", "smaller than", "less than", "newer than") else "gt"
        found.add((direction, *quantity_identity(m[2])))
    return found


def semantic_safe(text, rewrite):
    if rewrite.strip().upper() == "REFUSE":
        return True
    source, output = unquoted(text).lower(), unquoted(rewrite).lower()
    if re.search(r"\b(?:excluding|except|apart from|without|encrypt|upload|email|schedule)\b", source):
        return False
    action_words = {word for words in FAMILIES.values() for word in words} | {"upload", "email", "undo"}
    if re.search(r"(?:\b(?:and|then|also)\s+|;\s*)(?:(?:then|also)\s+)?(?:" + "|".join(sorted(action_words)) + r")\b", output):
        return False
    family = action_family(text)
    if family is None or family != action_family(rewrite):
        return False
    if constraints(text) != constraints(rewrite):
        return False
    for phrase in re.findall(r"\b(?:this|last)\s+(?:week|month|year)\b|\btoday\b|\byesterday\b", source):
        if phrase not in output:
            return False
    return True
