# Releasing

Releases are built, signed, and notarized on a Mac, then published to GitHub with `gh`. Every installed copy checks the release's `latest.json` once a day and offers the update.

## What you need

- **The update signing key**, at `~/.tauri/headroom.key`. Every update has to be signed with it: the app only installs updates signed with the key that matches the public key in [`tauri.conf.json`](src-tauri/tauri.conf.json). It never goes in the repo. Back it up somewhere safe, like a password manager. If it's lost, nobody who already has Headroom installed can get updates, and they'll have to download the new version by hand.
- **A "Developer ID Application" certificate** in your keychain, for signing the app. Without one, the app still builds, but it's unsigned, and people have to click **Open Anyway** the first time.
- **An App Store Connect API key** (a `.p8` file, its key ID, and your issuer ID), for notarizing.

## Cutting a release

1. Bump the version in `package.json`, `package-lock.json` (Headroom's own two entries), `src-tauri/Cargo.toml`, and Headroom's entry in `src-tauri/Cargo.lock`.

2. Build the universal app and the disk image, signed, with the update bundle:

   ```bash
   APPLE_SIGNING_IDENTITY="Developer ID Application: …" \
   APPLE_API_KEY=<key id> APPLE_API_ISSUER=<issuer id> APPLE_API_KEY_PATH=<path to the .p8> \
   TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/headroom.key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
   npx tauri build --target universal-apple-darwin --bundles app,dmg --config src-tauri/tauri.release.conf.json
   ```

   The app comes out notarized. Everything's in `src-tauri/target/universal-apple-darwin/release/bundle`.

3. Notarize and staple the disk image too:

   ```bash
   xcrun notarytool submit dmg/Headroom_<version>_universal.dmg --key <path to the .p8> --key-id <key id> --issuer <issuer id> --wait
   xcrun stapler staple dmg/Headroom_<version>_universal.dmg
   ```

4. Write `latest.json`, which the app checks for updates:

   ```json
   {
     "version": "<version>",
     "notes": "What's new, in a sentence.",
     "pub_date": "<now, like 2026-09-27T21:36:00Z>",
     "platforms": {
       "darwin-aarch64": { "signature": "<contents of macos/Headroom.app.tar.gz.sig>", "url": "https://github.com/dylan-hepworth/headroom/releases/download/v<version>/Headroom.app.tar.gz" },
       "darwin-x86_64": { "…the same…" },
       "darwin-universal": { "…the same…" }
     }
   }
   ```

5. Commit the version, tag it, push both, and publish the release with all four files:

   ```bash
   git commit -am "Version <version>" && git tag v<version> && git push origin main v<version>
   gh release create v<version> --verify-tag --latest --title "Headroom v<version>" --notes "…" \
     dmg/Headroom_<version>_universal.dmg macos/Headroom.app.tar.gz macos/Headroom.app.tar.gz.sig latest.json
   ```

Once it's up, everyone on an older version gets an update notification within a day.
