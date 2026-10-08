# lablet docs

User-facing documentation lives here.

- [Telemetry](telemetry.md): what lablet emits, where the `lablet` command sends it, how a library host provides OpenTelemetry, and the rules its telemetry contract follows, with the generated reference under [telemetry/](telemetry/README.md).

The examples are described in [../examples/](../examples/README.md): a two-turn fake-provider run, a collector with Jaeger to send a run to, and `traced_run`, a host with an SDK of its own that runs lablet as a library. The written guides (getting started, the config reference generated from the schema, and testing a framework with `provider-fake`) arrive in [build-plan](../../product/build-plan.md) phase 11.
