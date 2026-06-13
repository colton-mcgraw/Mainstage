#!/usr/bin/env python3
"""wordcount — a reference Mainstage plugin returning file metrics.

A slightly richer example than `greet`: multiple methods, the `int` return type,
file I/O (the plugin runs with the script's directory as its working directory, so
relative paths resolve there), and structured error reporting via `err`.
"""
import json
import sys


METHODS = [
    {
        "name": "lines",
        "params": [{"name": "path", "type": "string", "required": True}],
        "returns": "int",
    },
    {
        "name": "words",
        "params": [{"name": "path", "type": "string", "required": True}],
        "returns": "int",
    },
    {
        "name": "chars",
        "params": [{"name": "path", "type": "string", "required": True}],
        "returns": "int",
    },
]


def describe():
    return {"name": "wordcount", "methods": METHODS}


def _read(path):
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def call(method, args):
    path = args[0]["value"]["value"]
    try:
        text = _read(path)
    except OSError as e:
        # A failed call returns `err`; Mainstage surfaces it as an evaluation error
        # carrying the call's source span.
        return {"err": f"could not read '{path}': {e.strerror}"}

    if method == "lines":
        count = len(text.splitlines())
    elif method == "words":
        count = len(text.split())
    elif method == "chars":
        count = len(text)
    else:
        return {"err": f"unknown method '{method}'"}
    return {"ok": {"type": "int", "value": count}}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = json.loads(line)
        op = req.get("op")
        if op == "describe":
            resp = describe()
        elif op == "call":
            resp = call(req.get("method", ""), req.get("args", []))
        else:
            resp = {"err": f"unknown op '{op}'"}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
