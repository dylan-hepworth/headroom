# Releasing

Releases are built by GitHub Actions ([release.yml](.github/workflows/release.yml)) whenever a `v*` tag is pushed. The workflow builds a universal `.dmg`, publishes the release, and uploads the `latest.json` that the app checks for updates.

## One-time setup

### Update signing key (required)

Every update has to be signed, and the app only installs updates signed with the key that matches the public key in [`tauri.conf.json`](src-tauri/tauri.conf.json). The private key lives at `~/.tauri/headroom.key` and never goes in the repo.

Add it as a repo secret:

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/headroom.key
```

The key doesn't have a password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` can be left unset.

Back the key up somewhere safe, like a password manager. If it's lost, nobody who already has Headroom installed can get updates, and they'll have to download the new version by hand.

### Apple signing (optional)

Without this, releases still build, but they're unsigned, so people have to click **Open Anyway** the first time. With a paid Apple Developer account, add these secrets and the workflow will sign and notarize the app:

| Secret | What it is |
| --- | --- |
| `APPLE_CERTIFICATE` | Your "Developer ID Application" certificate exported as a `.p12`, then base64 encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | The certificate's name, e.g. `Developer ID Application: Dylan Hepworth (TEAMID)` |
| `APPLE_ID` | Your Apple ID email |
| `APPLE_PASSWORD` | An app-specific password from account.apple.com, not your real password |
| `APPLE_TEAM_ID` | Your 10-character team ID |

## Cutting a release

The version lives in `package.json` (`tauri.conf.json` reads it from there). `npm version` bumps it, commits, and tags in one go:

```bash
npm version patch
git push --follow-tags
```

Use `minor` or `major` instead of `patch` when it makes sense. Once the workflow finishes, the release is live, and everyone on an older version gets an update notification within a day.
