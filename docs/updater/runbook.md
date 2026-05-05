# Sparkle Release Runbook

## Preconditions

- Working tree reviewed and tests passing.
- `.env.sparkle.local` exists locally and is not tracked.
- Sparkle public key, feed URL, signing identity, notarization profile, and R2 credentials are configured.
- `dx`, `cargo`, `aws`, and macOS signing/notary tools are installed for macOS releases.
- Windows builds are packaged separately and distributed manually until Windows auto-update is implemented.

## Standard Release

1. Confirm version intent.
2. Run `scripts/smoke-core-workflow.sh`.
3. Run `scripts/smoke-agent-api-workflow.sh` (headless; covers auth-disabled and auth-enabled Agent API flows).
4. If Docker is available, optionally run `scripts/smoke-docker-sandbox-port-reuse.sh`.
5. Run `scripts/release-single-binary.sh`.
6. Verify the generated archive in `release-artifacts/`.
7. Verify `appcast.xml` and uploaded updater assets in the configured R2 bucket.
8. If `PUBLISH_GITHUB_RELEASE=true`, verify the GitHub release artifact.

## Dry Release Check

Use this before a real publish:

```bash
scripts/release-single-binary.sh --bump none --skip-notarize --skip-upload
```

This exercises version parsing, Sparkle setup, macOS bundle preparation, archive creation, and local appcast generation without notarizing or uploading.

## R2 Sync

The release pipeline syncs existing updater history from R2 by default before appcast generation. Disable that with:

```bash
scripts/release-single-binary.sh --skip-r2-sync
```

Skip upload while still generating local appcast files:

```bash
scripts/release-single-binary.sh --skip-upload
```

## Failure Handling

- If signing fails, verify `LOOPBOX_RELEASE_IDENTITY`.
- If notarization fails, verify `LOOPBOX_NOTARY_PROFILE`.
- If Sparkle setup fails, run `scripts/bootstrap-sparkle.sh --help` and confirm Sparkle is installed or `AUTO_INSTALL_SPARKLE=true`.
- If upload fails, verify R2 credentials and `CLOUDFLARE_R2_ENDPOINT`.
