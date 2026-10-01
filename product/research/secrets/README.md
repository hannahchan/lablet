# Secrets

How lablet's secrets are defined, withheld and cut, pressure-tested against the deployments that run it. The design is settled in `product/decisions.md` ("Secrets are derived from the config, withheld from every child, and cut once before the model", 2026-10-01) and written into spec §6 and §7.

- [stories.md](stories.md) — three deployment stories, each written to the exact config and the exact command the framework runs, with what the rules make of it: a CI runner, a laptop behind a gateway, and a cluster where the framework sets `OTEL_EXPORTER_OTLP_HEADERS`.
