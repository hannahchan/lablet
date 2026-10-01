# Three deployment stories, walked to the launch line

Written before the code of phase 6, as the build plan asks, so that each rule of the secrets design is checked against a config a framework would write and the command it would run. A story that needed an awkward config would have shown here; none did, with one caveat on the second (phase 8).

The rules each story is held to (`product/decisions.md`, 2026-10-01):

- A secret is defined by the config's fields: the model's key; every value of `telemetry.otlp.headers` and of an HTTP server's `headers`, written or substituted; the user information of `model.base_url`, `telemetry.otlp.endpoint` and a server's `url`; and every variable substituted into `tools.builtin.env` or a stdio server's `env`. Nothing is found by a name's pattern.
- A variable in the set is withheld from every command, and reaches a child only where its `env` names it. The one exception is the OTLP header variables the exporter reads (`OTEL_EXPORTER_OTLP_HEADERS` and its per-signal forms): inherited by every child, and cut.
- The value and its bounded parts are cut out of every tool result and every executor's error message before the model, the transcript or telemetry sees it: each line of a multi-line value and its `\n`-escaped form, the remainder after `Bearer `, `Basic ` or `Token `, each over 16 bytes. Encoded or altered copies aren't cut.
- Header values and URL credentials leave the config digest; `tools.builtin.env` stays in it.
- `check` prints the names it will withhold and the names whose values it will cut, never a value.

## Story 1: a CI runner, `GITHUB_TOKEN` and `gh` in every task

`lablet.yaml`:

```yaml
run:
  transcript_path: artifacts/{run_id}.transcript.json
model:
  provider: anthropic
  name: claude-sonnet-5
  # api_key_env left out: ANTHROPIC_API_KEY is read, withheld from every command, cut everywhere.
prompt:
  system_file: prompts/fix-the-failing-test.md
tools:
  builtin:
    root: /home/runner/work/repo/repo
    enabled: [bash, read_file, write_file]
    env:
      # Substituted into env, so a secret: withheld as inherited, handed to bash by this name,
      # cut from every result. gh reads GITHUB_TOKEN. Without this line the runner's token
      # would reach bash uncut, as the framework's own (decisions.md, 2026-09-30).
      GITHUB_TOKEN: ${GITHUB_TOKEN}
telemetry:
  otlp:
    endpoint: https://otel.example.internal:4317
    headers:
      # A header value: withheld from every command, cut everywhere, out of the digest.
      Authorization: Bearer ${OTEL_TOKEN}
  file:
    path: artifacts/telemetry.otlp.jsonl
```

The workflow step:

```yaml
- run: lablet run --config lablet.yaml --prompt-file task.md
  env:
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
    OTEL_TOKEN: ${{ secrets.OTEL_TOKEN }}
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

What holds: `env` in a task shows `GITHUB_TOKEN=[secret withheld]` and neither of the other two; `gh pr list` works; `echo $GITHUB_TOKEN` is `[secret withheld]`; `curl -d "$OTEL_TOKEN"` sends nothing, since the variable isn't there. The header's whole value, `Bearer <token>`, is registered beside the token, so a `curl -v` that echoes the header line shows `Authorization: [secret withheld]`. `lablet check` ends:

```
withheld: ANTHROPIC_API_KEY, GITHUB_TOKEN, OTEL_TOKEN
cut: ANTHROPIC_API_KEY, GITHUB_TOKEN, OTEL_TOKEN
passed: 3 tools
```

Digest: two runs whose `OTEL_TOKEN` rotated share a digest, as do two whose `Authorization` was written literally and rotated; two whose `tools.builtin.env` differs don't.

## Story 2: a laptop, a gateway `base_url`, one stdio server

`lablet.yaml`:

```yaml
model:
  provider: anthropic
  name: claude-sonnet-5
  api_key_env: GATEWAY_KEY
  # The URL's user information is a secret: GATEWAY_PASS withheld, the password cut
  # everywhere, the credentials left out of the digest. The host stays in it.
  base_url: https://hannah:${GATEWAY_PASS}@llm-gateway.corp.example/anthropic
prompt:
  system: You fix the failing test and stop.
tools:
  builtin:
    root: .
    enabled: [bash, read_file, write_file]
  mcp:
    - name: tracker
      transport: stdio
      command: tracker-mcp
      env:
        # Phase 8: this server alone gets it; bash doesn't; cut everywhere.
        TRACKER_TOKEN: ${TRACKER_TOKEN}
telemetry:
  file:
    # ${HOME} is substituted and is no secret: told, never found.
    path: ${HOME}/lablet-runs/telemetry.otlp.jsonl
```

`lablet.env` (1Password references, never values):

```
GATEWAY_KEY=op://Work/llm-gateway/key
GATEWAY_PASS=op://Work/llm-gateway/password
TRACKER_TOKEN=op://Work/tracker/token
```

The command:

```
op run --env-file=lablet.env -- lablet run --config lablet.yaml --prompt-file task.md
```

What holds: bash inherits `HOME`, `PATH`, the proxy, the toolchain; not `GATEWAY_KEY`, `GATEWAY_PASS` or `TRACKER_TOKEN`. A `curl -v` to the gateway that echoes the URL shows `https://hannah:[secret withheld]@llm-gateway…`, since the password is registered on its own beside the whole `hannah:<password>`. `lablet check` ends:

```
withheld: GATEWAY_KEY, GATEWAY_PASS, TRACKER_TOKEN
cut: GATEWAY_KEY, GATEWAY_PASS, TRACKER_TOKEN
passed: 3 tools
```

Caveat: `tools.mcp` is refused until phase 8 (`Unsupported::McpServers`), by `check` too, so the story runs whole only then; the secret set covers a stdio server's `env` from phase 6, so phase 8 inherits it. The phase 7 adapter owes `without_url` on its errors, since reqwest's Display carries the URL whole (the panel's D2).

## Story 3: a cluster, where the framework sets `OTEL_EXPORTER_OTLP_HEADERS`

`/config/lablet.yaml`:

```yaml
model:
  provider: anthropic
  name: claude-sonnet-5
prompt:
  system_file: /config/system.md
tools:
  builtin:
    root: /work
    enabled: [bash, read_file, write_file]
telemetry:
  # Endpoint, protocol and headers come from the environment (decisions.md, 2026-10-01).
  # Stating an endpoint here would drop the environment's headers.
  otlp: {}
  file:
    path: /work/out/telemetry.otlp.jsonl
```

The pod:

```yaml
containers:
  - name: lablet
    image: ghcr.io/example/lablet:0.1.0
    command: [
      "lablet",
      "run",
      "--config",
      "/config/lablet.yaml",
      "--prompt-file",
      "/task/prompt.md",
    ]
    env:
      - name: ANTHROPIC_API_KEY
        valueFrom: { secretKeyRef: { name: lablet-secrets, key: anthropic-api-key } }
      - name: OTEL_EXPORTER_OTLP_ENDPOINT
        value: https://otel-collector.observability.svc:4317
      - name: OTEL_EXPORTER_OTLP_HEADERS
        # Authorization=Bearer%20<token>
        valueFrom: { secretKeyRef: { name: lablet-secrets, key: otlp-headers } }
```

What holds: bash inherits `OTEL_EXPORTER_OTLP_HEADERS`, the one inherited class, and not `ANTHROPIC_API_KEY`. `env` shows `OTEL_EXPORTER_OTLP_HEADERS=Authorization=Bearer%20[secret withheld]`: the whole value, each decoded value (`Bearer <token>`) and the remainder after `Bearer` are registered, and the remainder is what the percent-encoded form still holds. A token that itself holds percent-escapes would show encoded, which is the accepted residual beside sending it by name: `curl -H "$OTEL_EXPORTER_OTLP_HEADERS"` sends it. A framework secret that no child should have is left out of lablet's environment: `env -u DEPLOY_KEY lablet run …`. `lablet check` ends `withheld: ANTHROPIC_API_KEY` and `cut: ANTHROPIC_API_KEY, OTEL_EXPORTER_OTLP_HEADERS`.

The OTLP header class is read by the round that resolves the OpenTelemetry environment (phase 6, round 2); round 1 leaves its place in the derived set, as a name that's cut and not withheld.
