# Verification samples

These files were supplied as the local test corpus. The generator verification
uses the 88 `.dbc`/`.DBC` files recursively; the other supplied formats are
retained alongside them. The original sample contents have not been modified.

- `cantools/`: fixtures from [cantools](https://github.com/cantools/cantools).
  Its MIT license is included in `cantools/LICENSE`.
- `model3/`: [model3dbc](https://github.com/joshwardell/model3dbc), with its
  original README and MIT license.
- `example.dbc` and `envvars.dbc`: additional user-supplied test inputs.

The supplied snapshots have no recorded upstream revision. Verification results
in `artifacts/sample-report.json` identify the exact DBC inputs by SHA-256.
See `docs/node-simulation-codegen.ko.md` for commands and supported behavior.
