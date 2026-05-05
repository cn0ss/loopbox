# Sparkle Key Management

Date: 2026-02-22

## Scope

This document defines handling rules for Sparkle EdDSA keys used to sign update metadata and archives.

## Rules

- Never commit private Sparkle keys to git.
- Keep private keys only in approved operator keychain/secret storage.
- Public key may be stored in release automation and `Info.plist`.
- Restrict update publish access to designated maintainers.

## Recommended setup

1. Generate Sparkle keys once with Sparkle tooling (`generate_keys`).
2. Record public key in release variables and pass to `--sparkle-public-key`.
3. Store private key in secure secret manager or local keychain with access controls.
4. Rotate key only with coordinated client migration plan.

## Operational controls

- Require two-person review for updater infrastructure changes.
- Audit every appcast publish event (who, when, version, hash/signature).
- Keep immutable history of published appcast snapshots for rollback.

## Incident response

If private key compromise is suspected:

1. Stop publishing updates immediately.
2. Rotate Sparkle keys and prepare updated app with new public key.
3. Publish security advisory and migration instructions.
4. Rebuild release provenance from audit logs.
