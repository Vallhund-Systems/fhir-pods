# CLI terminology fixture

`diagnostics-status.txt` is a byte-identical copy of
`conformance/terminology/diagnostics-status.txt`, delivered in `009e37b`.
It contains the complete nine-code IPS 2.0.1 diagnostic-report-status set,
without display text. The original fetch/compiler provenance is in the
planning repository's `reviews/terminology/` report.

Tests accept `final`, reject `entered-in-error`, and check the exact unresolved
need before loading and its absence afterward. The fixture is local; CLI tests
make no terminology-server requests.
