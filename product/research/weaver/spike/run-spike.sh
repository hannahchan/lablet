#!/usr/bin/env bash
# Commands run in the weaver spike, in order. Run from this directory with weaver 0.26.1 on PATH.
# Vendor the dependencies first (paths in manifest.yaml are relative to the working directory):
#   git clone --depth 1 --branch v1.44.0 https://github.com/open-telemetry/semantic-conventions.git /tmp/semconv
#   git clone https://github.com/open-telemetry/semantic-conventions-genai.git /tmp/genai && git -C /tmp/genai checkout c88d504ab3d9879f8e50d3cc87e69775e11db234
#   mkdir -p deps/semantic-conventions deps/semantic-conventions-genai
#   cp -R /tmp/semconv/model deps/semantic-conventions/model && cp -R /tmp/genai/model deps/semantic-conventions-genai/model
set -euo pipefail
WEAVER=${WEAVER:-weaver}
PKG=https://github.com/open-telemetry/opentelemetry-weaver-packages.git@587265869c37180940d3bae29d23ca385e6baa00

# 1. Validate the registry (v2 output for policies) with the lablet policy and the shared naming/stability packages.
$WEAVER registry check --v2 -r registry -p policies \
  -p "$PKG[policies/check/naming_conventions]" -p "$PKG[policies/check/stability]"

# 2. Generate the Rust crate (v2 context) and prove it compiles.
$WEAVER registry generate --v2 -r registry -t templates rust generated/lablet-telemetry-registry
( cd generated/lablet-telemetry-registry && cargo build && cargo clippy --all-targets -- -D warnings && cargo doc --no-deps )

# 3. Generate markdown docs with the shared docs package; embed snippets in a hand-written page.
$WEAVER registry generate --v2 -r registry -t "$PKG[templates/docs]" --param registry_base_url=/docs/telemetry markdown docs
$WEAVER registry update-markdown --v2 -r registry -t "$PKG[templates/docs]" --target markdown --param registry_base_url=/docs/telemetry docs-handwritten

# 4. Live-check: v1 mode (no --v2) so attributes referenced from dependencies are indexed; stop via the admin endpoint.
$WEAVER registry live-check -r registry --config .weaver.toml --format json --output http \
  --otlp-grpc-port 4317 --admin-port 4320 --inactivity-timeout 60 > live-check.log 2>&1 &
PID=$!
until curl -fsS http://127.0.0.1:4320/health >/dev/null; do sleep 0.5; done
( cd otlp-client && cargo run --quiet )          # or: weaver registry emit -r registry --skip-policies
curl -sS -X POST http://127.0.0.1:4320/stop -o live-check-report.json
wait $PID || echo "live-check exit=$? (1 = violations found)"
