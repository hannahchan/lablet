"""The primary reference in its reference configuration, run once per call.

The pinned script of claude-code.md's "Reference configuration", with the
changes read-capture.md and the phase 7 design settled: tool search off, so
every definition goes up front; the stand-in server `lab` in place of the
server under test, offering `bash`, `read_file` and `write_file` with
lablet's own descriptions and input schemas, each run over the working
directory as lablet's built-ins run over their root; the body log on, to
`--out/<run>/<model>`, with the OTel logs exporter pointed at the sink on
127.0.0.1:4318; and a prompt for each of the runs R0 to R7.

This script never reads the key. The SDK merges `env` over the shell's, so
`ANTHROPIC_API_KEY` reaches the CLI from the shell that exported it, and
under `--base-url` a dummy is set over it so the real one never reaches the
mock upstream. Nothing here reads a value of the environment; the stand-in's
`bash` drops variables by name alone.

It needs the pinned SDK, `claude-agent-sdk==0.2.159`, and Python 3.10 or
later, which the SDK asks for. Run it with
`python3 -I capture.py --run R2 --model claude-opus-5-5 --out bodies`.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import signal
import subprocess
import sys

from claude_agent_sdk import ClaudeAgentOptions, create_sdk_mcp_server, query, tool

HERE = os.path.dirname(os.path.abspath(__file__))
RUNS = ["R0", "R1", "R2", "R3", "R4", "R5", "R6", "R7"]

# A key of no key's shape, so redact.py's search for `sk-ant-` can't trip
# on the mock run's own files; the mock reads no header anyway.
DUMMY_KEY = "dummy-key-for-the-mock-upstream-never-a-real-one"

# How long a call may take, as lablet's built-ins are offered by default
# (`tools.builtin.timeout`, 120s), and the suffix lablet's executor adds to
# every description.
CALL_LIMIT_SECONDS = 120
TIMEOUT_SUFFIX = " A call that takes longer than 120s is stopped."

# Variables a command never gets from the stand-in, by name: lablet drops the
# variable that filled its key, and the context variables are its own.
DROPPED_FROM_COMMANDS = ("ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "TRACEPARENT", "TRACESTATE", "BAGGAGE", "B3")
DROPPED_PREFIXES = ("OTEL_", "X_B3_", "CLAUDE_")

# lablet/crates/adapters/secondary/tools-builtin/src/bash.rs, DESCRIPTION
BASH_DESCRIPTION = (
    "Runs a command with `bash -c` and returns what it wrote: standard "
    "output and standard error together, in the order they were written, then one line with the "
    "exit code, as `exit code: 0`. A command that exits with another code is a result like any "
    "other. Every command is a new process that starts in the run's root directory, so a working "
    "directory, a variable or a function that one command set isn't there for the next: write "
    "`cd dir && command` as one command. The call returns when the command has exited and nothing "
    "it started holds its output open, so send the output of a process that's to stay in the "
    "background somewhere else, as in `server > server.log 2>&1 &`. A command that runs for as "
    "long as a call may take is killed, with every process it started."
)
BASH_SCHEMA = {
    "type": "object",
    "properties": {
        "command": {
            "type": "string",
            "description": "The command line, as it would be typed at a bash prompt.",
        }
    },
    "required": ["command"],
    "additionalProperties": False,
}

# lablet/crates/adapters/secondary/tools-builtin/src/read_file.rs, DESCRIPTION
READ_FILE_DESCRIPTION = (
    "Returns the text of a file. A path that isn't absolute starts at "
    "the run's root directory, which is where `bash` starts, and a path that leads outside that "
    "directory once symbolic links are followed is refused. `offset` and `limit` choose lines, so "
    "a file too long to be returned whole can be read in parts: `offset` is how many lines to "
    "skip, and `limit` is how many to return after them. Bytes that aren't UTF-8 are returned as "
    "U+FFFD."
)
READ_FILE_SCHEMA = {
    "type": "object",
    "properties": {
        "path": {"type": "string", "description": "The file to read."},
        "offset": {
            "type": "integer",
            "minimum": 0,
            "description": "How many lines to skip from the start of the file. None when left out.",
        },
        "limit": {
            "type": "integer",
            "minimum": 1,
            "description": "The most lines to return. Every line after the offset when left out.",
        },
    },
    "required": ["path"],
    "additionalProperties": False,
}

# lablet/crates/adapters/secondary/tools-builtin/src/write_file.rs, DESCRIPTION
WRITE_FILE_DESCRIPTION = (
    "Writes a text file: makes it, with the directories on the way to "
    "it that are missing, or replaces what it held. `content` is the whole of the file. A path "
    "that isn't absolute starts at the run's root directory, which is where `bash` starts, and a "
    "path that leads outside that directory once symbolic links are followed is refused."
)
WRITE_FILE_SCHEMA = {
    "type": "object",
    "properties": {
        "path": {"type": "string", "description": "The file to write."},
        "content": {"type": "string", "description": "The text the file is to hold."},
    },
    "required": ["path", "content"],
    "additionalProperties": False,
}

# One prompt per run; R2 and R3 read task.md, and R4 has two parts.
PROMPTS = {
    "R0": "Write hello.md holding the one line `hello`, read it back, and say what it holds.",
    "R1": "Reply with the one word `ready`.",
    "R4": {
        "text": (
            "Reply, in your message and not in a file, with an essay of about 1,500 words on how "
            "rivers shape the valleys they run through."
        ),
        "write": (
            "Write essay.md, a file of about 2,000 words on how rivers shape the valleys they run "
            "through, in one write_file call, then say that you wrote it."
        ),
    },
    "R5": "Reply with the one word `ready`.",
    "R6": (
        "Write big.txt: 400 lines, each holding the line's number, a space, and 48 letters x, so "
        "the file is about 20 KB. Then read big.txt whole with read_file, with no offset and no "
        "limit, forty times, one call at a time, each after the previous result has come back; "
        "don't shorten the reads and don't use bash for them. Then say done."
    ),
    "R7": "Reply with nothing.",
}


def result(text, is_error=False):
    out = {"content": [{"type": "text", "text": text}]}
    if is_error:
        out["is_error"] = True
    return out


def make_lab_server(root):
    """The stand-in server over `root`, the directory the reference runs in."""

    def reachable(path, verb):
        """The resolved path under `root`, or the refusal lablet's root gives."""
        candidate = path if os.path.isabs(path) else os.path.join(root, path)
        resolved = os.path.realpath(candidate)
        root_resolved = os.path.realpath(root)
        if resolved != root_resolved and not resolved.startswith(root_resolved + os.sep):
            raise ValueError("%s wasn't %s: it leads outside the run's root directory" % (path, verb))
        return resolved

    @tool("bash", BASH_DESCRIPTION + TIMEOUT_SUFFIX, BASH_SCHEMA)
    async def bash(args):
        command = args["command"]
        environment = {
            name: value
            for name, value in os.environ.items()
            if name not in DROPPED_FROM_COMMANDS and not name.startswith(DROPPED_PREFIXES)
        }
        process = await asyncio.create_subprocess_exec(
            "bash",
            "-c",
            command,
            cwd=root,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            output, _ = await asyncio.wait_for(process.communicate(), CALL_LIMIT_SECONDS)
        except asyncio.TimeoutError:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            await process.wait()
            return result(
                "bash was stopped after %ds, the longest the call could take" % CALL_LIMIT_SECONDS,
                is_error=True,
            )
        text = output.decode("utf-8", errors="replace")
        if text and not text.endswith("\n"):
            text += "\n"
        code = process.returncode
        ended = ("exit code: %d" % code) if code >= 0 else ("killed by signal %d" % -code)
        return result(text + ended)

    @tool("read_file", READ_FILE_DESCRIPTION + TIMEOUT_SUFFIX, READ_FILE_SCHEMA)
    async def read_file(args):
        path = args["path"]
        offset = int(args.get("offset", 0) or 0)
        limit = args.get("limit")
        try:
            resolved = reachable(path, "read")
            if not os.path.isfile(resolved):
                return result("%s wasn't read: it isn't a file" % path, is_error=True)
            with open(resolved, "rb") as handle:
                text = handle.read().decode("utf-8", errors="replace")
        except (OSError, ValueError) as error:
            message = str(error) if isinstance(error, ValueError) else "%s wasn't read: %s" % (path, error)
            return result(message, is_error=True)
        if offset or limit is not None:
            lines = text.splitlines(keepends=True)
            end = None if limit is None else offset + int(limit)
            text = "".join(lines[offset:end])
        return result(text)

    @tool("write_file", WRITE_FILE_DESCRIPTION + TIMEOUT_SUFFIX, WRITE_FILE_SCHEMA)
    async def write_file(args):
        path = args["path"]
        content = args["content"]
        try:
            resolved = reachable(path, "written")
            if os.path.exists(resolved) and not os.path.isfile(resolved):
                return result("%s wasn't written: it isn't a file" % path, is_error=True)
            os.makedirs(os.path.dirname(resolved), exist_ok=True)
            with open(resolved, "wb") as handle:
                handle.write(content.encode("utf-8"))
        except (OSError, ValueError) as error:
            message = str(error) if isinstance(error, ValueError) else "%s wasn't written: %s" % (path, error)
            return result(message, is_error=True)
        return result("wrote %d bytes to %s" % (len(content.encode("utf-8")), path))

    return create_sdk_mcp_server(name="lab", version="1.0.0", tools=[bash, read_file, write_file])


def options(args, bodies_dir):
    """The pinned reference configuration, with the design's changes."""
    SRV, TS = "lab", False
    env = {
        "ENABLE_TOOL_SEARCH": "true" if TS else "false",
        "CLAUDE_CONFIG_DIR": args.config_dir,
        "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
        "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1",
        "CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS": "1",
        "CLAUDE_AGENT_SDK_DISABLE_BUILTIN_AGENTS": "1",
        "CLAUDE_CODE_DISABLE_BUNDLED_SKILLS": "1",
        "CLAUDE_CODE_DISABLE_POLICY_SKILLS": "1",
        "ENABLE_CLAUDEAI_MCP_SERVERS": "false",
        "DISABLE_AUTOUPDATER": "1",
        "DISABLE_GROWTHBOOK": "1",
        "DISABLE_TELEMETRY": "1",
        "DISABLE_ERROR_REPORTING": "1",
        "CLAUDE_CODE_SIMPLE_SYSTEM_PROMPT": "0",
        "MCP_SDK_GENERATION": "v2",
        "MCP_PROTOCOL_NEGOTIATION": "legacy",
        "CLAUDE_CODE_PROMPT_CACHE_TTL": "5m",
        "MAX_MCP_OUTPUT_TOKENS": "25000",
        "CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY": "10",
        "CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS": "0",
        "DISABLE_AUTO_COMPACT": "1",
        "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
        "OTEL_LOG_RAW_API_BODIES": "file:" + bodies_dir,
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_METRICS_EXPORTER": "none",
        "OTEL_EXPORTER_OTLP_PROTOCOL": "http/protobuf",
        "OTEL_EXPORTER_OTLP_ENDPOINT": args.sink,
    }
    # R1 reads the attribution block with and without `=0`, so the pin is
    # optional there; every other run keeps it.
    if not args.with_attribution:
        env["CLAUDE_CODE_ATTRIBUTION_HEADER"] = "0"
    if args.base_url:
        env["ANTHROPIC_BASE_URL"] = args.base_url
        env["ANTHROPIC_API_KEY"] = DUMMY_KEY
    with open(os.path.join(HERE, "prompt.md"), "r", encoding="utf-8") as handle:
        system_prompt = handle.read()
    settings = {
        "system_prompt": system_prompt,
        "tools": ["ToolSearch"] if TS else [],
        "mcp_servers": {SRV: make_lab_server(args.cwd)},
        "strict_mcp_config": True,
        "allowed_tools": ["mcp__%s__*" % SRV],
        "permission_mode": "dontAsk",
        "setting_sources": [],
        "skills": [],
        "model": args.model,
        "cwd": args.cwd,
        "extra_args": {"disable-slash-commands": None},
        "env": env,
    }
    # The pinned script states `effort="high"`; left out, the model's own
    # default applies, which is what R1 reads. Pass `--effort high` for the
    # pinned value.
    if args.effort:
        settings["effort"] = args.effort
    return ClaudeAgentOptions(**settings)


def prompt_for(run, part):
    if run in ("R2", "R3"):
        with open(os.path.join(HERE, "task.md"), "r", encoding="utf-8") as handle:
            return handle.read()
    prompt = PROMPTS[run]
    if isinstance(prompt, dict):
        return prompt[part]
    return prompt


async def one_query(args, bodies_dir, prompt):
    os.makedirs(bodies_dir, exist_ok=True)
    log_path = os.path.join(bodies_dir, "messages.txt")
    with open(log_path, "a", encoding="utf-8") as log:
        async for message in query(prompt=prompt, options=options(args, bodies_dir)):
            line = repr(message)
            print(line, flush=True)
            log.write(line + "\n")


async def main_async(args):
    os.makedirs(args.cwd, exist_ok=True)
    os.makedirs(args.config_dir, exist_ok=True)
    base = os.path.join(os.path.abspath(args.out), args.run, args.model)
    parts = [None]
    if isinstance(PROMPTS.get(args.run), dict):
        parts = sorted(PROMPTS[args.run].keys())
    for part in parts:
        bodies_dir = base if part is None else os.path.join(base, part)
        print("capture: %s %s -> %s" % (args.run, args.model, bodies_dir), flush=True)
        await one_query(args, bodies_dir, prompt_for(args.run, part))


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--run", required=True, choices=RUNS)
    parser.add_argument("--model", required=True, help="the full model id, such as claude-opus-5-5")
    parser.add_argument("--out", required=True, help="the body log's directory; a run writes under <out>/<run>/<model>")
    parser.add_argument("--base-url", default=None, help="R5: the mock upstream, with a dummy key set over the shell's")
    parser.add_argument("--cwd", default="/private/tmp/lab-capture/cwd", help="the reference's working directory, under no user name")
    parser.add_argument("--config-dir", default="/private/tmp/lab-capture/config", help="a fresh CLAUDE_CONFIG_DIR")
    parser.add_argument("--sink", default="http://127.0.0.1:4318", help="the OTLP listener")
    parser.add_argument("--effort", default=None, choices=["low", "medium", "high", "xhigh", "max"])
    parser.add_argument("--with-attribution", action="store_true", help="R1: leave CLAUDE_CODE_ATTRIBUTION_HEADER unset")
    args = parser.parse_args(argv)
    asyncio.run(main_async(args))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
