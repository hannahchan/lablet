#!/usr/bin/env bash
# The phase 7 capture of the primary reference, the sitting of the README's
# "The commands": `all` runs the whole of it, two to three hours, and each
# step runs alone too. Export ANTHROPIC_API_KEY in this shell first. The
# script only checks that the variable is set: the SDK merges capture.py's
# env over the shell's, so the key reaches Claude Code from this shell and
# nothing here reads it. It never installs Python, and it never prints the
# environment.
set -euo pipefail

# The bodies hold whole conversations, and /private/tmp is open to every
# local user, so what the script makes is readable by the one who runs it.
umask 077

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The working directory's path goes up in every request body, and the bodies
# are handed over, so the default has no user name in it.
CAPTURE_HOME="${LAB_CAPTURE_HOME:-/private/tmp/lab-capture}"
VENV="$CAPTURE_HOME/venv"
PY="$VENV/bin/python3"
BODIES="$CAPTURE_HOME/bodies"
CWD="$CAPTURE_HOME/cwd"
CONFIG="$CAPTURE_HOME/config"
LOGS="$CAPTURE_HOME/logs"

# Not OTLP's own 4318: a collector already there, such as the one lablet's
# Jaeger examples run, would take the reference's logs in place of the sink.
SINK_PORT="${LAB_SINK_PORT:-4418}"
MOCK_PORT="${LAB_MOCK_PORT:-8765}"

# The pins of the design's section g: the SDK, and the Claude Code its wheel
# bundles. A capture of another version is a capture of something else.
SDK_VERSION=0.2.159
CLI_VERSION=2.1.281

STARTER=claude-opus-5-5
PREVIOUS=claude-sonnet-5
R1_MODELS=(claude-sonnet-5 claude-sonnet-5-5 claude-opus-5-5 claude-haiku-5-5 claude-fable-5-1)
SEQUENCES=(529x12 500x3-then-200 429-retry-after-7-then-200 429-retry-after-61
  500-x-should-retry-false 400-prompt-too-long-then-200)

# The processes this script started. Cleanup stops these and nothing else.
SINK_PID=""
MOCK_PID=""
# The R5 body directory of the pair in progress, named if Ctrl-C cuts it short.
R5_STAGING=""
# One line per capture that failed or didn't run, for the table at the end.
FAILURES=()
# Where capture() writes, and a word added to its label. r1-attribution and
# r5 change them around their own calls.
OUT="$BODIES"
NOTE=""
# The directory whose presence refuses a capture, when it isn't the run's own.
CHECK_DIR=""

usage() {
  cat <<EOF
usage: run-capture.sh <step>
       run-capture.sh r1 [model...]
       run-capture.sh r4 [text|write]
       run-capture.sh r5 [sequence...]

Export ANTHROPIC_API_KEY in this shell first. Everything lands under
$CAPTURE_HOME (LAB_CAPTURE_HOME).

steps:
  all               setup, r0, r1, r2r3, r2-sonnet, r4, r5, r6, r7, handover
  setup             the venv with claude-agent-sdk==$SDK_VERSION; no key needed
  r0                the smoke: one prompt, then a check that no key reached the bodies
  r1 [model...]     one request on each of the five models, or on those named
  r2r3              R2 from an empty cwd, then R3 at once
  r2-sonnet         R2 on $PREVIOUS, from an empty cwd
  r4 [text|write]   the output cap, CLAUDE_CODE_MAX_OUTPUT_TOKENS=300; both parts, or one
  r5 [sequence...]  the mock upstream, every sequence or those named; no key needed
  r6                forty-odd reads of about 20 KB, compaction off
  r7                "Reply with nothing."
  handover          redact.py over the bodies, then the one path to send
  r1-attribution    R1 on $STARTER with the attribution header; not in all

models: ${R1_MODELS[*]}
sequences: ${SEQUENCES[*]}
EOF
}

# cwd/ is removed before each R2, so the home must never put it at the root.
check_settings() {
  case "$CAPTURE_HOME" in
    /*[!/]*) ;;
    *)
      echo "error: LAB_CAPTURE_HOME must be an absolute path other than /" >&2
      exit 2
      ;;
  esac
  local port
  for port in "$SINK_PORT" "$MOCK_PORT"; do
    case "$port" in
      '' | *[!0-9]*)
        echo "error: LAB_SINK_PORT and LAB_MOCK_PORT take a port number" >&2
        exit 2
        ;;
    esac
  done
}

need_key() {
  if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
    echo "error: export ANTHROPIC_API_KEY in this shell first" >&2
    exit 1
  fi
}

# Each of these, set in this shell, would change the capture: ANTHROPIC_BASE_URL
# sends it elsewhere, ANTHROPIC_AUTH_TOKEN is a second credential the CLI would
# send, to R5's mock too, the two USE_ variables pick another provider, and the
# output cap would hold every run to R4's. capture.py and r4 set the one a run
# needs for that run alone. Presence alone is tested, and only the name printed.
refuse_stray_env() {
  local name found=0
  for name in ANTHROPIC_BASE_URL ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_USE_BEDROCK \
    CLAUDE_CODE_USE_VERTEX CLAUDE_CODE_MAX_OUTPUT_TOKENS; do
    if [ -n "${!name:-}" ]; then
      echo "error: $name is set in this shell; unset it for the capture" >&2
      found=1
    fi
  done
  if [ "$found" -eq 1 ]; then
    exit 1
  fi
}

# Whether something accepts a connection on 127.0.0.1:PORT. The system's
# python3 is enough here, as for the sink, the mock and redact.py: they're
# standard library alone, and only capture.py needs the venv.
port_answers() {
  python3 -I -c '
import socket, sys
probe = socket.socket()
probe.settimeout(0.5)
sys.exit(0 if probe.connect_ex(("127.0.0.1", int(sys.argv[1]))) == 0 else 1)
' "$1"
}

# wait_for_port PORT PID WHAT LOG: returns once WHAT, started as PID, listens,
# and stops the sitting if it exits first or takes more than about five seconds.
wait_for_port() {
  local tries=0
  until port_answers "$1"; do
    if ! kill -0 "$2" 2>/dev/null; then
      echo "error: $3 exited before it listened on 127.0.0.1:$1; see $4" >&2
      exit 1
    fi
    tries=$((tries + 1))
    if [ "$tries" -ge 25 ]; then
      echo "error: $3 didn't listen on 127.0.0.1:$1 within 5s; see $4" >&2
      exit 1
    fi
    sleep 0.2
  done
}

# A script's background jobs ignore SIGINT, so Ctrl-C never reaches the sink
# or the mock, and TERM is what stops them.
stop_process() {
  if [ -n "$1" ]; then
    kill -TERM "$1" 2>/dev/null || true
    wait "$1" 2>/dev/null || true
  fi
}

failed() {
  FAILURES+=("$(printf '%-48s %s' "$1" "$2")")
}

# The failures the design foresees, so the table tells them from the rest.
expected_for() {
  case "$1" in
    "R1 claude-fable-5-1")
      echo "expected on an organisation whose retention is under 30 days"
      ;;
    "R5 "*" 529x12" | "R5 "*" 429-retry-after-61" | "R5 "*" 500-x-should-retry-false")
      echo "expected: the sequence never reaches a 200"
      ;;
    "R7 "*)
      echo "possibly expected: R7 asks for an empty reply"
      ;;
  esac
}

report_failures() {
  if [ "${#FAILURES[@]}" -eq 0 ]; then
    return 0
  fi
  local line marked=0
  echo
  echo "Failed or not run (${#FAILURES[@]}):"
  for line in "${FAILURES[@]}"; do
    echo "  $line"
    case "$line" in
      *expected*) marked=1 ;;
    esac
  done
  if [ "$marked" -eq 1 ]; then
    echo "A failure marked expected is a fact to record, not a bug."
  fi
  echo "Rerun a step alone once the cause of its failure is fixed. A capture not run"
  echo "since its directory exists ran in an earlier invocation: remove the directory"
  echo "to run it again."
}

on_exit() {
  local status=$?
  stop_process "${MOCK_PID:-}"
  stop_process "${SINK_PID:-}"
  if [ -n "${R5_STAGING:-}" ] && [ -e "$R5_STAGING" ]; then
    echo "note: $R5_STAGING is left from the R5 pair cut short; remove it before r5 runs again" >&2
  fi
  report_failures
  exit "$status"
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Started by the first capture of an invocation, so setup and handover never
# need the port, and every run of `all` shares the one sink.
need_sink() {
  if [ -n "$SINK_PID" ]; then
    return 0
  fi
  if [ ! -x "$PY" ]; then
    echo "error: there's no venv at $VENV; run setup first: $0 setup" >&2
    exit 1
  fi
  mkdir -p "$BODIES" "$LOGS"
  if port_answers "$SINK_PORT"; then
    echo "error: something already listens on 127.0.0.1:$SINK_PORT; set LAB_SINK_PORT to a free port" >&2
    exit 1
  fi
  python3 -I "$HERE/otlp_sink.py" --port "$SINK_PORT" >>"$LOGS/sink.log" 2>&1 &
  SINK_PID=$!
  wait_for_port "$SINK_PORT" "$SINK_PID" "the OTLP sink" "$LOGS/sink.log"
  echo "[ok] OTLP sink on 127.0.0.1:$SINK_PORT, its log in $LOGS/sink.log"
}

# capture RUN MODEL [ARG...]: one capture.py call, with the shared directories
# and the sink. A failure is recorded and returned, never fatal, so one model's
# error doesn't end a sitting of hours. Its output stays on the terminal: the
# SDK's messages are how a run is watched.
capture() {
  local run="$1" model="$2"
  shift 2
  local label="$run $model${NOTE:+ $NOTE}"
  # capture.py adds to a run's directory, so a second attempt would mix with
  # the first in the bodies the replay and the skeletons read. R5 has a rule
  # of its own, in r5_pair.
  local kept="${CHECK_DIR:-$OUT/$run/$model}"
  if [ "$run" != R5 ] && [ -e "$kept" ]; then
    echo "[FAIL] $label: $kept exists; remove it to rerun"
    failed "$label" "not run: $kept exists"
    return 1
  fi
  need_sink
  echo "[..] $label"
  local start status=0
  start=$(date +%s)
  "$PY" -I "$HERE/capture.py" --run "$run" --model "$model" --out "$OUT" --cwd "$CWD" \
    --config-dir "$CONFIG" --sink "http://127.0.0.1:$SINK_PORT" "$@" || status=$?
  local took=$(($(date +%s) - start))
  if [ "$status" -eq 0 ]; then
    echo "[ok] $label (${took}s)"
    return 0
  fi
  echo "[FAIL] $label (${took}s): capture.py exited $status"
  local expected
  expected="$(expected_for "$label")"
  failed "$label" "capture.py exited $status${expected:+; $expected}"
  return 1
}

# The first Python 3.10 or later on PATH, which the SDK asks for. macOS's own
# python3 is 3.9.
find_python() {
  local name path
  for name in python3.13 python3.12 python3.11 python3.10 python3; do
    path="$(command -v "$name")" || continue
    if "$path" -I -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then
      echo "$path"
      return 0
    fi
  done
  return 1
}

setup() {
  echo "[..] setup under $CAPTURE_HOME"
  mkdir -p "$BODIES" "$CWD" "$CONFIG" "$LOGS"
  # A venv made from macOS's own python3, as the README's earlier commands
  # made it, is there but too old, and pip's refusal wouldn't say why.
  if [ ! -x "$PY" ] || ! "$PY" -I -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then
    local base
    if ! base="$(find_python)"; then
      echo "error: claude-agent-sdk $SDK_VERSION needs Python 3.10 or later, and there's none on PATH. Install one with:" >&2
      echo "  brew install python@3.12" >&2
      echo "then run setup again." >&2
      exit 1
    fi
    echo "[..] venv at $VENV, from $base"
    # --clear, since a venv whose interpreter is gone, after an upgrade of
    # its base Python, is no venv to keep.
    "$base" -I -m venv --clear "$VENV"
    echo "[ok] venv"
  fi
  # pip leaves the pinned version as it is when it's there, so setup can run again.
  echo "[..] claude-agent-sdk==$SDK_VERSION"
  "$PY" -I -m pip install -q --disable-pip-version-check "claude-agent-sdk==$SDK_VERSION"
  local versions sdk cli
  versions="$("$PY" -I -c 'import claude_agent_sdk, claude_agent_sdk._cli_version as bundled; print(claude_agent_sdk.__version__, bundled.__cli_version__)')"
  read -r sdk cli <<<"$versions"
  if [ "$sdk" != "$SDK_VERSION" ] || [ "$cli" != "$CLI_VERSION" ]; then
    echo "error: expected claude-agent-sdk $SDK_VERSION with Claude Code $CLI_VERSION, found $sdk with $cli" >&2
    exit 1
  fi
  echo "[ok] claude-agent-sdk $sdk, Claude Code $cli"
}

# The smoke. Nothing more runs when it fails: a sitting of hours on a broken
# setup spends money for nothing. An R0 that passed its checks is checked again
# and not paid for again, so `all` run again after a sitting cut short resumes
# it. The marker tells that R0 from one that failed, which leaves its directory
# too, since capture.py makes it before the first request.
r0() {
  local dir="$BODIES/R0/$STARTER" passed="$LOGS/R0.passed"
  if [ -e "$dir" ] && [ -e "$passed" ]; then
    echo "[skip] R0 $STARTER: $dir passed its checks in an earlier run, so only the checks run again"
  elif [ -e "$dir" ]; then
    echo "error: $dir is there from an R0 that didn't pass its checks; remove it, then run again" >&2
    exit 1
  elif ! capture R0 "$STARTER"; then
    echo "error: R0 failed, so nothing more runs; its messages are in $dir/messages.txt." >&2
    echo "Remove $dir before you run it again." >&2
    exit 1
  fi
  if ! compgen -G "$BODIES/R0/*/*.request.json" >/dev/null; then
    echo "error: R0 left no request body under $BODIES/R0, so the body log is off; nothing more runs" >&2
    exit 1
  fi
  # A key in the body log means every later run writes one too, and redact.py
  # would refuse the handover anyway.
  local holding
  if holding="$(grep -rl sk-ant- "$BODIES")"; then
    echo "error: these files hold a string of a key's shape, so nothing more runs:" >&2
    echo "$holding" >&2
    echo "Remove them (rm -rf \"$BODIES\"), send nothing, and find out why a key reached a body before running anything again." >&2
    exit 1
  fi
  touch "$passed"
  echo "[ok] R0 wrote request bodies, and no key reached them"
}

# Every model, or the ones named, so one can be run again alone. capture
# records a failure itself, and `|| true` keeps the sitting going: Fable 5.1's
# 400 is a fact to record.
r1() {
  local models=("$@") model
  if [ "${#models[@]}" -eq 0 ]; then
    models=("${R1_MODELS[@]}")
  fi
  for model in "${models[@]}"; do
    capture R1 "$model" || true
  done
}

# The oldest or the newest modification time, in seconds, of the request
# bodies directly in DIR, or nothing when there are none. stat -f is macOS's.
request_time() {
  local which="$1" file mtime best=""
  for file in "$2"/*.request.json; do
    if [ ! -e "$file" ]; then
      continue
    fi
    mtime="$(stat -f %m "$file")"
    if [ -z "$best" ] || { [ "$which" = oldest ] && [ "$mtime" -lt "$best" ]; } ||
      { [ "$which" = newest ] && [ "$mtime" -gt "$best" ]; }; then
      best="$mtime"
    fi
  done
  echo "$best"
}

# Each R2 starts from an empty working directory, as the replay runs lablet on
# an empty root, so a `ls` in the task sees the same files on both sides. R3
# keeps R2's files and reads the cache entry R2 left, which lives five minutes
# from the last request that touched it, so R3 runs at once and only on the R2
# just before it: the two run together or not at all.
r2r3() {
  local r2="$BODIES/R2/$STARTER" r3="$BODIES/R3/$STARTER"
  if [ -e "$r2" ] || [ -e "$r3" ]; then
    echo "[FAIL] R2 and R3 $STARTER: an earlier attempt is there, and R3 needs a fresh R2; remove both $r2 and $r3, then run r2r3 again"
    failed "R2 $STARTER" "not run: an earlier R2 or R3 is there"
    failed "R3 $STARTER" "not run: an earlier R2 or R3 is there"
    return 0
  fi
  rm -rf "$CWD"
  if ! capture R2 "$STARTER"; then
    echo "[skip] R3 $STARTER: R3 is R2 again on R2's cache entry and files, and R2 failed"
    failed "R3 $STARTER" "not run: R2 failed"
    return 0
  fi
  capture R3 "$STARTER" || true
  # From R2's last request to R3's first, so R3's own start-up counts too.
  local last first
  last="$(request_time newest "$r2")"
  first="$(request_time oldest "$r3")"
  if [ -z "$last" ] || [ -z "$first" ]; then
    return 0
  fi
  if [ "$((first - last))" -lt 300 ]; then
    echo "[ok] R3's first request came $((first - last))s after R2's last; the cache entry lives 300s"
  else
    echo "[FAIL] R3's first request came $((first - last))s after R2's last, past the cache entry's 300s; remove both $r2 and $r3, then run r2r3 again"
    failed "R3 $STARTER" "came $((first - last))s after R2, past the cache entry's 300s"
  fi
}

# The previous generation, for the contrast in the reminder channel.
r2_sonnet() {
  rm -rf "$CWD"
  capture R2 "$PREVIOUS" || true
}

# The cap is in the environment of this one call: the SDK merges capture.py's
# env over the shell's, so it reaches the CLI. capture.py runs R4's two parts
# in order and goes on past the first's error, which the capped text ends in;
# a part named here runs alone, into its own directory, after the other failed.
r4() {
  local part="${1:-}"
  if [ -n "$part" ]; then
    CHECK_DIR="$BODIES/R4/$STARTER/$part"
    NOTE="$part"
    CLAUDE_CODE_MAX_OUTPUT_TOKENS=300 capture R4 "$STARTER" --part "$part" || true
    CHECK_DIR=""
    NOTE=""
  else
    CLAUDE_CODE_MAX_OUTPUT_TOKENS=300 capture R4 "$STARTER" || true
  fi
}

known_sequence() {
  local known
  for known in "${SEQUENCES[@]}"; do
    if [ "$known" = "$1" ]; then
      return 0
    fi
  done
  return 1
}

known_model() {
  local known
  for known in "${R1_MODELS[@]}"; do
    if [ "$known" = "$1" ]; then
      return 0
    fi
  done
  return 1
}

# One R5 pair: the mock plays SEQUENCE, the reference runs on MODEL against it
# with a dummy key, and the pair's bodies are kept apart from the next pair's.
# In the foreground with no watchdog, so Ctrl-C reaches the capture.
r5_pair() {
  local sequence="$1" model="$2"
  local label="R5 $model $sequence"
  local staging="$BODIES/R5/$model"
  local kept="$BODIES/R5/$model--$sequence"
  if [ -e "$kept" ]; then
    echo "[FAIL] $label: $kept exists; remove it to rerun"
    failed "$label" "not run: $kept exists"
    return 0
  fi
  # capture.py writes every sequence of a model to the one directory, so what
  # a pair cut short left there would merge into this pair.
  if [ -e "$staging" ]; then
    echo "error: $staging is left from an R5 pair cut short; remove it, or rename it to $model--<sequence>, then run r5 again" >&2
    exit 1
  fi
  need_sink
  # A mock left running would serve its own sequence in place of this one.
  if port_answers "$MOCK_PORT"; then
    echo "error: something already listens on 127.0.0.1:$MOCK_PORT, the mock's port; stop it, or set LAB_MOCK_PORT to a free port" >&2
    exit 1
  fi
  # The mock's timestamps show the waits between retries, so its log goes in
  # with the bodies.
  mkdir -p "$staging"
  R5_STAGING="$staging"
  python3 -I "$HERE/mock_upstream.py" --sequence "$sequence" --port "$MOCK_PORT" >"$staging/mock.log" 2>&1 &
  MOCK_PID=$!
  wait_for_port "$MOCK_PORT" "$MOCK_PID" "the mock upstream" "$staging/mock.log"
  # The log is a file, so nothing on the terminal moves while the reference
  # waits between retries; a new line in it means the reference still retries.
  echo "[..] the mock plays $sequence; watch its requests with: tail -f \"$staging/mock.log\""
  case "$sequence" in
    529x12)
      echo "[..] 529x12 may take a few minutes: the reference backs off between its ten retries, and those waits are the evidence"
      ;;
    429-retry-after-61)
      echo "[..] 429-retry-after-61 may take about ten minutes: the reference honours each retry-after of 61s, and those waits are the evidence"
      ;;
  esac
  NOTE="$sequence"
  capture R5 "$model" --base-url "http://127.0.0.1:$MOCK_PORT" || true
  NOTE=""
  stop_process "$MOCK_PID"
  MOCK_PID=""
  mv "$staging" "$kept"
  R5_STAGING=""
  echo "     kept in $kept, the mock's log in mock.log"
}

# Every sequence, or the ones named. 529x12 runs on both generations, as the
# design asks; the rest on the starter's model.
r5() {
  local sequences=("$@") sequence
  if [ "${#sequences[@]}" -eq 0 ]; then
    sequences=("${SEQUENCES[@]}")
  fi
  for sequence in "${sequences[@]}"; do
    r5_pair "$sequence" "$STARTER"
    if [ "$sequence" = 529x12 ]; then
      r5_pair "$sequence" "$PREVIOUS"
    fi
  done
}

r6() {
  capture R6 "$STARTER" || true
}

r7() {
  capture R7 "$STARTER" || true
}

# R1 without the attribution pin, for the "with and without =0" reading. Its
# own --out keeps these bodies apart from the pinned R1's.
r1_attribution() {
  OUT="$BODIES/attribution"
  NOTE="with attribution"
  capture R1 "$STARTER" --with-attribution || true
  OUT="$BODIES"
  NOTE=""
  # Not in all, so it may come after the handover, which these bodies missed.
  echo "note: run handover after this, so these bodies are redacted with the rest"
}

# The capture stays where it was made, and is read in place once redact.py
# has run, so the one thing to send is its path.
handover() {
  if [ ! -d "$BODIES" ]; then
    echo "error: there's no $BODIES to hand over; run the capture first" >&2
    exit 1
  fi
  echo "[..] handover: redact.py over $BODIES"
  local status=0
  python3 -I "$HERE/redact.py" "$BODIES" --cwd "$CWD" || status=$?
  case "$status" in
    0)
      echo "[ok] handover. The one path to send: $BODIES"
      ;;
    2)
      echo "error: a file named above holds a string of a key's shape; nothing was changed, and nothing is to be sent" >&2
      echo "Remove them (rm -rf \"$BODIES\"), send nothing, and find out why a key reached a body before running anything again." >&2
      exit 1
      ;;
    *)
      echo "error: redact.py exited $status" >&2
      exit 1
      ;;
  esac
}

main() {
  local step="${1:-}"
  if [ "$#" -gt 0 ]; then
    shift
  fi
  case "$step" in
    -h | --help | help)
      usage
      exit 0
      ;;
    r1)
      local model
      for model in "$@"; do
        if ! known_model "$model"; then
          echo "error: R1 has no model $model" >&2
          usage >&2
          exit 2
        fi
      done
      ;;
    r5)
      local sequence
      for sequence in "$@"; do
        if ! known_sequence "$sequence"; then
          echo "error: there's no sequence $sequence" >&2
          usage >&2
          exit 2
        fi
      done
      ;;
    r4)
      case "${1:-}" in
        '' | text | write) ;;
        *)
          echo "error: R4 has the parts text and write, not $1" >&2
          usage >&2
          exit 2
          ;;
      esac
      if [ "$#" -gt 1 ]; then
        usage >&2
        exit 2
      fi
      ;;
    all | setup | r0 | r2r3 | r2-sonnet | r6 | r7 | handover | r1-attribution)
      if [ "$#" -gt 0 ]; then
        usage >&2
        exit 2
      fi
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  check_settings
  # Before anything is spent. R5 talks to the mock alone, with a dummy key.
  case "$step" in
    setup | handover) ;;
    r5) refuse_stray_env ;;
    *)
      need_key
      refuse_stray_env
      ;;
  esac
  case "$step" in
    all)
      setup
      r0
      r1
      r2r3
      r2_sonnet
      r4
      r5
      r6
      r7
      handover
      ;;
    setup) setup ;;
    r0) r0 ;;
    r1) r1 "$@" ;;
    r2r3) r2r3 ;;
    r2-sonnet) r2_sonnet ;;
    r4) r4 "$@" ;;
    r5) r5 "$@" ;;
    r6) r6 ;;
    r7) r7 ;;
    handover) handover ;;
    r1-attribution) r1_attribution ;;
  esac
  if [ "${#FAILURES[@]}" -gt 0 ]; then
    exit 1
  fi
}

main "$@"
