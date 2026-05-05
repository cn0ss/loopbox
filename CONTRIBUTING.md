# Contributing to Loopbox

Thanks for contributing.

## Contribution License

By submitting a contribution, you agree that your contribution is licensed under the project's [PolyForm Noncommercial License 1.0.0](LICENSE) and that the project author retains the right to offer commercial licenses for the combined work.

## Sign-Off Requirement (DCO)

All commits must be signed off.

Use:

```bash
git commit -s
```

The sign-off indicates your agreement with the Developer Certificate of Origin (DCO).

## Local Smoke Check

Before opening a pull request, run:

```bash
scripts/smoke-core-workflow.sh
```

The smoke script checks formatting, runs the Rust test suite, runs clippy with warnings denied, and builds the app.
