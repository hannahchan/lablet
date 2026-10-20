"""The handover step: redacts a capture in place, and refuses one with a key.

Over every file under the directory given, in this order:

1. Looks for the string `sk-ant-`, the shape of an Anthropic key. One found
   anywhere ends the run with exit code 2, naming the file and nothing of
   its text, and no file is changed.
2. Drops the top-level `metadata` key of every `.json` body and of every
   line of a `.jsonl` file, since it may carry an account or session id.
3. Replaces the working directory's path, as given and as the system
   resolves it, with `<CWD>` in every text file, so no path of the machine
   stays in the bodies.

Standard library only. Run it with
`python3 -I redact.py <directory> [--cwd /private/tmp/lab-capture/cwd]`.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

KEY_SHAPE = b"sk-ant-"


def files_under(root):
    for directory, _, names in os.walk(root):
        for name in sorted(names):
            yield os.path.join(directory, name)


def without_metadata(text, path):
    """`text` with the top-level `metadata` dropped from the JSON document,
    or from each line of a `.jsonl`, and the number of keys dropped."""
    dropped = 0
    if path.endswith(".jsonl"):
        lines = []
        for line in text.splitlines():
            if not line.strip():
                lines.append(line)
                continue
            try:
                document = json.loads(line)
            except ValueError:
                lines.append(line)
                continue
            if isinstance(document, dict) and "metadata" in document:
                del document["metadata"]
                dropped += 1
            lines.append(json.dumps(document, ensure_ascii=False))
        return "\n".join(lines) + ("\n" if text.endswith("\n") else ""), dropped
    try:
        document = json.loads(text)
    except ValueError:
        return text, 0
    if isinstance(document, dict) and "metadata" in document:
        del document["metadata"]
        dropped = 1
    return json.dumps(document, ensure_ascii=False, indent=2) + "\n", dropped


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directory")
    parser.add_argument("--cwd", default="/private/tmp/lab-capture/cwd")
    args = parser.parse_args(argv)

    paths = list(files_under(args.directory))
    holding_a_key = []
    for path in paths:
        with open(path, "rb") as handle:
            if KEY_SHAPE in handle.read():
                holding_a_key.append(path)
    if holding_a_key:
        for path in holding_a_key:
            print("redact: %s holds a string of a key's shape; nothing was changed" % path)
        return 2

    spellings = []
    for spelling in (args.cwd, os.path.realpath(args.cwd), os.path.abspath(args.cwd)):
        spelling = spelling.rstrip("/")
        if spelling and spelling not in spellings:
            spellings.append(spelling)
    # The longest first, so a prefix of another spelling never cuts it short.
    spellings.sort(key=len, reverse=True)

    metadata_dropped = 0
    paths_replaced = 0
    for path in paths:
        with open(path, "rb") as handle:
            raw = handle.read()
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
        changed = text
        if path.endswith((".json", ".jsonl")):
            changed, dropped = without_metadata(changed, path)
            metadata_dropped += dropped
        for spelling in spellings:
            count = changed.count(spelling)
            if count:
                paths_replaced += count
                changed = changed.replace(spelling, "<CWD>")
        if changed != text:
            with open(path, "wb") as handle:
                handle.write(changed.encode("utf-8"))
    print(
        "redact: %d files read, no key found, %d metadata keys dropped, %d paths replaced"
        % (len(paths), metadata_dropped, paths_replaced)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
