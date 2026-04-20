# Security — Code Signing & CI Pipeline

This document describes the secrets and credentials required for the HQ Sync release pipeline.

## GitHub Secrets

The following secrets must be configured in the repository settings under **Settings > Secrets and variables > Actions**.

### Apple Code Signing

| Secret | Description |
|--------|-------------|
| `APPLE_CERTIFICATE` | Base64-encoded `.p12` Developer ID Application certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12` |

#### How to create the certificate

1. In **Keychain Access**, find your "Developer ID Application" certificate (issued by Apple).
2. Right-click the certificate (with its private key) and select **Export**.
3. Save as `.p12` with a strong password.
4. Base64-encode it:
   ```bash
   base64 -i DeveloperID.p12 | pbcopy
   ```
5. Paste the base64 string as the `APPLE_CERTIFICATE` secret.
6. Set `APPLE_CERTIFICATE_PASSWORD` to the password you chose in step 3.

### Apple Notarization

| Secret | Description |
|--------|-------------|
| `APPLE_API_KEY` | App Store Connect API key ID (e.g. `ABC123DEF4`) |
| `APPLE_API_ISSUER` | App Store Connect API issuer UUID |
| `APPLE_API_KEY_PATH` | Contents of the `.p8` API key file (the raw text, not a file path) |

#### How to create the API key

1. Go to [App Store Connect > Users and Access > Integrations > Team Keys](https://appstoreconnect.apple.com/access/integrations/api).
2. Click **Generate API Key** with the "Developer" role.
3. Note the **Key ID** (this is `APPLE_API_KEY`) and the **Issuer ID** at the top of the page (this is `APPLE_API_ISSUER`).
4. Download the `.p8` key file. You can only download it once.
5. Copy the full contents of the `.p8` file and paste it as the `APPLE_API_KEY_PATH` secret.

> The CI workflow writes this secret to a temporary file at runtime and deletes it after the build.

### Tauri Auto-Updater Signing

| Secret | Description |
|--------|-------------|
| `TAURI_SIGNING_PRIVATE_KEY` | Ed25519 private key for signing update bundles |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the signing key |

#### How to generate updater keys

```bash
npx @tauri-apps/cli signer generate -- -w ~/.tauri/hq-sync.key
```

This produces:
- `~/.tauri/hq-sync.key` — private key (set as `TAURI_SIGNING_PRIVATE_KEY`)
- `~/.tauri/hq-sync.key.pub` — public key (embed in `tauri.conf.json` under `plugins.updater.pubkey`)

Set the password you chose during generation as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

### GitHub Token

`GITHUB_TOKEN` is automatically provided by GitHub Actions. No manual setup needed. It is used to create releases and upload artifacts.

## Manual Signing & Notarization

The `scripts/` directory contains standalone scripts for local signing and packaging:

- **`scripts/notarize.sh <path-to.app>`** — Submit a `.app` to Apple's notary service, wait for approval, and staple the ticket. Requires `APPLE_API_KEY`, `APPLE_API_ISSUER`, and `APPLE_API_KEY_PATH` environment variables.

- **`scripts/create-dmg.sh <path-to.app> <output.dmg>`** — Create a compressed DMG with an Applications symlink for drag-and-drop installation.

## Security Notes

- The Developer ID certificate private key never leaves the CI runner. It is imported into a temporary keychain that is deleted after the build.
- The App Store Connect API key `.p8` file is written to a temporary path and deleted in the cleanup step.
- All secrets are masked in GitHub Actions logs.
- The Tauri updater signature ensures end users can verify that updates come from us before applying them.
- DMG and `.app` bundles are signed with a Developer ID certificate, which means macOS Gatekeeper will allow users to open the app without security warnings.
