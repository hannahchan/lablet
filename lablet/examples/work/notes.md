# Notes

The collector listens on two ports, 14317 for gRPC and 14318 for HTTP, so another process can hold the defaults.

Jaeger keeps traces and no logs, so the wide event is read from the collector's output.
