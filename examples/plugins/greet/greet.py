#!/usr/bin/env python3
"""greet — a minimal reference Mainstage plugin.

The smallest useful plugin: a single string method. It speaks the
newline-delimited JSON protocol over stdio — one request per line in, one
response line out. See docs/PLUGINS.md for the full specification.
"""
import json
import sys

METHODS = [
    {
        "name": "hello",
        "params": [{"name": "name", "type": "string", "required": True}],
        "returns": "string",
    },
]


def describe():
    return {"name": "greet", "methods": METHODS}


def call(method, args):
    if method == "hello":
        who = args[0]["value"]["value"]
        return {"ok": {"type": "string", "value": f"hello, {who}"}}
    return {"err": f"unknown method '{method}'"}


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
