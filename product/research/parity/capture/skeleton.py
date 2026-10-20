"""Reduces Messages API request bodies to their skeletons, for a diff.

A skeleton keeps what the parity rows read and drops every text: the
top-level parameters with their values, `metadata` as its keys alone since
its value is an account's or a session's id, `system` as a list of block types,
`tools` as masked names with the byte sizes of their descriptions and
schemas, every `cache_control` object where it sits, and `messages` as
roles and blocks in order, each block as its type with its text replaced by
its byte length, a `tool_use` input as its keys in the order they arrived
and its size, a `thinking` block as its size and whether it carries a
signature. Tool names are masked by their position in the request's `tools`
array, `T1` onward, with any `mcp__<server>__` prefix taken off first, so the
reference's `mcp__lab__bash` and lablet's `bash` land on the same mask.

Standard library only. Run it with
`python3 -I skeleton.py <file or directory>... [--out <dir>]`; a directory
stands for every `*.request.json` in it, in name order, and not in its
subdirectories, so a capture's `lablet-replay/` isn't mixed into the
reference's side. Each skeleton is printed as JSON, or written as
`<name>.skeleton.json` under `--out`. A document that is no request body, one
with neither `model` nor `messages` at the top level, is refused by its keys,
so a wrapper around the body is seen at R0 and not reduced as if it were one.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys

MCP_PREFIX = re.compile(r"^mcp__[^_]+(?:_[^_]+)*__")


def size(text):
    return len(text.encode("utf-8")) if isinstance(text, str) else len(json.dumps(text))


def with_marker(skeleton, block):
    if isinstance(block, dict) and "cache_control" in block:
        skeleton["cache_control"] = block["cache_control"]
    return skeleton


def block_skeleton(block, masks):
    if isinstance(block, str):
        return {"type": "text", "bytes": size(block)}
    kind = block.get("type", "?")
    skeleton = {"type": kind}
    if kind == "text":
        skeleton["bytes"] = size(block.get("text", ""))
    elif kind == "thinking":
        skeleton["bytes"] = size(block.get("thinking", ""))
        skeleton["signature"] = bool(block.get("signature"))
    elif kind == "redacted_thinking":
        skeleton["bytes"] = size(block.get("data", ""))
    elif kind == "tool_use":
        name = MCP_PREFIX.sub("", block.get("name", ""))
        skeleton["name"] = masks.get(name, "T?")
        inputs = block.get("input", {})
        skeleton["input_keys"] = list(inputs.keys()) if isinstance(inputs, dict) else "?"
        skeleton["input_bytes"] = size(inputs)
    elif kind == "tool_result":
        skeleton["is_error"] = bool(block.get("is_error", False))
        content = block.get("content", "")
        if isinstance(content, list):
            skeleton["content"] = [block_skeleton(inner, masks) for inner in content]
        else:
            skeleton["bytes"] = size(content)
    else:
        skeleton["bytes"] = size(block)
    return with_marker(skeleton, block)


class NotARequest(ValueError):
    pass


def skeleton_of(body):
    if not isinstance(body, dict) or not ("model" in body or "messages" in body):
        keys = sorted(body) if isinstance(body, dict) else type(body).__name__
        raise NotARequest("not a request body; its top-level keys are %s" % (keys,))
    tools = body.get("tools", [])
    masks = {}
    for index, tool in enumerate(tools):
        name = MCP_PREFIX.sub("", tool.get("name", ""))
        masks.setdefault(name, "T%d" % (index + 1))

    parameters = {}
    for key in sorted(body):
        if key in ("system", "tools", "messages"):
            continue
        value = body[key]
        if key == "metadata" and isinstance(value, dict):
            value = sorted(value)
        parameters[key] = value

    system = body.get("system")
    if isinstance(system, str):
        system_skeleton = [{"type": "text", "bytes": size(system)}]
    elif isinstance(system, list):
        system_skeleton = [block_skeleton(block, masks) for block in system]
    else:
        system_skeleton = None

    tools_skeleton = []
    for tool in tools:
        name = MCP_PREFIX.sub("", tool.get("name", ""))
        skeleton = {
            "name": masks.get(name, "T?"),
            "description_bytes": size(tool.get("description", "")),
            "schema_bytes": size(tool.get("input_schema", {})),
        }
        for key in sorted(tool):
            if key not in ("name", "description", "input_schema", "cache_control"):
                skeleton[key] = tool[key]
        tools_skeleton.append(with_marker(skeleton, tool))

    messages_skeleton = []
    for message in body.get("messages", []):
        content = message.get("content", "")
        if isinstance(content, list):
            blocks = [block_skeleton(block, masks) for block in content]
        else:
            blocks = [block_skeleton(content, masks)]
        entry = {"role": message.get("role", "?"), "content": blocks}
        for key in sorted(message):
            if key not in ("role", "content"):
                entry[key] = message[key]
        messages_skeleton.append(entry)

    return {
        "parameters": parameters,
        "system": system_skeleton,
        "tools": tools_skeleton,
        "messages": messages_skeleton,
    }


def request_files(targets):
    for target in targets:
        if os.path.isdir(target):
            for name in sorted(os.listdir(target)):
                if name.endswith(".request.json") and os.path.isfile(os.path.join(target, name)):
                    yield os.path.join(target, name)
        else:
            yield target


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("targets", nargs="+")
    parser.add_argument("--out", help="write <name>.skeleton.json files here instead of printing")
    args = parser.parse_args(argv)
    if args.out:
        os.makedirs(args.out, exist_ok=True)
    for path in request_files(args.targets):
        with open(path, "r", encoding="utf-8") as handle:
            body = json.load(handle)
        try:
            skeleton = skeleton_of(body)
        except NotARequest as refused:
            print("%s: %s" % (path, refused), file=sys.stderr)
            return 2
        text = json.dumps(skeleton, indent=2, sort_keys=False)
        if args.out:
            name = os.path.basename(path)
            if name.endswith(".request.json"):
                name = name[: -len(".request.json")]
            with open(os.path.join(args.out, name + ".skeleton.json"), "w", encoding="utf-8") as out:
                out.write(text + "\n")
        else:
            print("# %s" % path)
            print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
