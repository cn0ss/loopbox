# Loopbox Updater

Loopbox uses Sparkle for macOS updates. All release tooling now lives in this public repo and builds the normal public app binary directly from this workspace.

## Local Configuration

Copy `.env.sparkle.local.example` to `.env.sparkle.local` and fill in signing, Sparkle, Cloudflare R2, and optional GitHub release settings. Do not commit `.env.sparkle.local`.

Required release variables:

- `LOOPBOX_RELEASE_IDENTITY`
- `LOOPBOX_NOTARY_PROFILE`
- `SPARKLE_PUBLIC_KEY`
- `SPARKLE_FEED_URL`
- `LOOPBOX_UPDATES_DIR`
- `CLOUDFLARE_R2_ACCOUNT_ID`
- `CLOUDFLARE_R2_BUCKET`
- `CLOUDFLARE_R2_ACCESS_KEY_ID`
- `CLOUDFLARE_R2_SECRET_ACCESS_KEY`

## Release Commands

Run the full macOS Sparkle/R2 flow:

```bash
scripts/release-single-binary.sh
```

Useful dry-run style release checks:

```bash
scripts/release-single-binary.sh --bump none --skip-notarize --skip-upload
scripts/release-sparkle-cloudflare.sh --skip-notarize --skip-upload
scripts/release-macos.sh --skip-build --no-notarize
```

Build only:

```bash
dx bundle --platform macos --package-types macos --release
```

If another `dx` binary is earlier in `PATH`, use `$HOME/.cargo/bin/dx` directly or set `DIOXUS_CLI_BIN` for the release scripts.

Windows packaging is handled separately and is not wired into the Sparkle auto-update flow yet:

```bat
scripts\release-windows.bat v0.3.0
```

## Notes

- There is no overlay repo, private feature crate, Polar env file, or `edition-ee` build feature.
- All features are present in the public app binary.
- Commercial use is handled by the public license/commercial terms, not by runtime activation.
- Windows builds currently use manual distribution. The in-app updater reports that Windows auto-update is unavailable.
