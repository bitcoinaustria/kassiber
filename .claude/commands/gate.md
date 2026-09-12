Run the full pre-push/PR gate defined in
[CONTRIBUTING.md](../../CONTRIBUTING.md#verification-and-review):

```sh
./scripts/quality-gate.sh
```

Report failures and incomplete checks; do not call the work push-ready unless
the gate passes.
