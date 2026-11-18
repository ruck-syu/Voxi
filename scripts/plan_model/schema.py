"""Versioned training target, independent of natural-language parsing.

This validates structure, not correspondence with a user's meaning. Passing it
does not authorize execution. The app planner and confirmation remain required.
"""
import json

VERSION = "voxi-plan-v2"
TYPES = ("pdfs", "images", "videos", "files", "documents", "screenshots")


def validate(value):
    if value == "REFUSE":
        return value
    if not isinstance(value, dict) or set(value) != {"steps", "schedule"}:
        raise ValueError("expected steps and schedule")
    steps = value["steps"]
    if not isinstance(steps, list) or not 1 <= len(steps) <= 8:
        raise ValueError("one to eight steps required")
    def literal(text):
        if not isinstance(text, str) or not text.strip() or len(text) > 256 or any(ord(c) < 32 for c in text):
            raise ValueError("invalid literal")
        return text
    for index, step in enumerate(steps):
        if not isinstance(step, list) or not step:
            raise ValueError("invalid step")
        op = step[0]
        if op == "mkdir" and len(step) == 3:
            literal(step[1]); literal(step[2])
            if any(c in step[1] for c in "/\\") or step[1] in (".", ".."):
                raise ValueError("folder name must be one component")
        elif op in ("move", "copy") and len(step) == 4 and step[1] in TYPES:
            literal(step[2])
            destination = step[3]
            if isinstance(destination, dict):
                if set(destination) != {"ref"} or type(destination["ref"]) is not int or not 0 <= destination["ref"] < index:
                    raise ValueError("invalid previous-step reference")
                if steps[destination["ref"]][0] != "mkdir":
                    raise ValueError("destination reference must name a created folder")
            else:
                literal(destination)
        elif op == "trash" and len(step) == 3 and step[1] in TYPES:
            literal(step[2])
        elif op == "rename" and len(step) == 4:
            for item in step[1:]: literal(item)
        else:
            raise ValueError("unsupported step")
    schedule = value["schedule"]
    if schedule is not None:
        if not isinstance(schedule, list) or not schedule:
            raise ValueError("invalid schedule")
        if schedule[0] == "repeat" and len(schedule) == 4:
            _, every, unit, clock = schedule
            if type(every) is not int or not 1 <= every <= 365 or unit not in ("minutes", "hours", "days", "weeks"):
                raise ValueError("invalid repeat")
            if unit == "minutes" and every < 15:
                raise ValueError("repeat too frequent")
            if clock is not None:
                if unit not in ("days", "weeks"): raise ValueError("clock needs days or weeks")
                validate_clock(clock)
        elif schedule[0] == "weekday" and len(schedule) == 3:
            if schedule[1] not in ("monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"):
                raise ValueError("invalid weekday")
            validate_clock(schedule[2])
        elif schedule[0] == "after" and len(schedule) == 3:
            if type(schedule[1]) is not int or not 1 <= schedule[1] <= 365 or schedule[2] not in ("minutes", "hours"):
                raise ValueError("invalid delay")
        elif schedule[0] == "at" and len(schedule) == 3:
            validate_clock(schedule[1])
            if schedule[2] not in ("next", "tomorrow"): raise ValueError("invalid date reference")
        else:
            raise ValueError("unsupported schedule")
    return value


def validate_clock(clock):
    if not isinstance(clock, str) or len(clock) != 5 or clock[2] != ":" or not (clock[:2] + clock[3:]).isdigit():
        raise ValueError("invalid clock")
    if int(clock[:2]) > 23 or int(clock[3:]) > 59: raise ValueError("invalid clock")


def encode(value):
    validate(value)
    if value == "REFUSE": return value
    steps = [[(["ref", item["ref"]] if isinstance(item, dict) else item) for item in step] for step in value["steps"]]
    # T5 already knows JSON brackets. Avoid newly initialized brace embeddings
    # at the first decoder position; the canonical in-memory target is unchanged.
    return json.dumps(["plan", steps, value["schedule"]], ensure_ascii=False, separators=(",", ":"))


def decode(text):
    if text.strip() == "REFUSE": return "REFUSE"
    wire = json.loads(text)
    if not isinstance(wire, list) or len(wire) != 3 or wire[0] != "plan" or not isinstance(wire[1], list):
        raise ValueError("expected version-2 plan array")
    steps = []
    for step in wire[1]:
        if not isinstance(step, list): raise ValueError("invalid step")
        steps.append([({"ref": item[1]} if isinstance(item, list) and len(item) == 2 and item[0] == "ref" else item) for item in step])
    return validate({"steps": steps, "schedule": wire[2]})
