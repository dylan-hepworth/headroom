# AGENTS.md

Instructions for coding agents working in this repo.

## When the user says "Go"

"Go", "build it", "install it", or anything like that means: build Headroom on this Mac and install it. Work through the steps below in order. Keep the user posted in short lines, and ask before installing anything on their machine.

### 1. Check the machine

Run `uname -s`. If it isn't `Darwin`, stop and tell the user Headroom only runs on macOS.

### 2. Check the tools

Check each of these. If something is missing, say what it is and ask before installing it.

| Tool | Check | Install if missing |
| --- | --- | --- |
| Xcode command line tools | `xcode-select -p` | `xcode-select --install` (this opens a dialog; wait until the user says it finished) |
| Rust | `cargo --version` | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y`, then `source "$HOME/.cargo/env"` |
| Node 18+ | `node --version` | `brew install node` if Homebrew is installed, otherwise send the user to https://nodejs.org |

If `cargo` still isn't found after installing Rust, run `source "$HOME/.cargo/env"` in the same shell as the build.

### 3. Build

From the repo root:

```bash
npm install
npm run build
```

This doesn't need the update signing key. That's only for published releases. The first build compiles a lot of Rust and takes a few minutes, so tell the user that before starting. The app ends up at `src-tauri/target/release/bundle/macos/Headroom.app`.

### 4. Install and launch

If Headroom is already running, quit it first:

```bash
osascript -e 'quit app "Headroom"'
```

If `/Applications/Headroom.app` already exists, it's an older build of this same app. Replace it:

```bash
rm -rf /Applications/Headroom.app
cp -R src-tauri/target/release/bundle/macos/Headroom.app /Applications/
open /Applications/Headroom.app
```

### 5. Tell the user what's next

Keep it short:

- Look for the little ring in the menu bar, with your usage next to it, and click it.
- For a login that doesn't expire, run `claude setup-token`, copy the token it prints, and paste it in under **Settings… → Sign In**. Without a token, Headroom uses the Claude Code login, which expires every few hours. The first time, macOS asks for keychain access; click Always Allow.
- A `⚠` means Headroom couldn't read usage. The menu says why.
- In **Settings… → Alerts**, click **Send Test Alert** once so macOS asks for notification permission.
- To start it at login, turn on **Open at login** in Settings.

## If the build fails

- Errors mentioning `xcrun`, the SDK, or the linker usually mean the command line tools are missing or broken. The user can fix that with `sudo xcode-select --reset`; it needs their password, so ask them to run it.
- `tauri: command not found` means `npm install` didn't finish. Run it again.
- If something else fails, show the user the first error, not the whole log.

## Project layout

- `src-tauri/src/main.rs`: most of the app (menu bar item, fetching usage, alerts, token storage, the settings window's commands)
- `src-tauri/src/hooks.rs`: adds and removes Headroom's Claude Code hooks in `~/.claude/settings.json`, and what the binary does when a hook runs it
- `src-tauri/src/sessions.rs`: keeps track of Claude Code sessions from what the hooks report
- `src-tauri/src/transcripts.rs`: reads Claude Code's transcripts for usage by project and the daily recap
- `src-tauri/src/context.rs`: how full each Claude Code conversation's context window is, for the "Context is filling up" alert
- `src-tauri/src/desktop.rs`: what the Claude app knows about its Code chats, so the menu bar's dots match the app's
- `src-tauri/src/wallpaper.rs`: the desktop wallpaper, for the menu bar preview in Settings
- `src/`: the settings window (React), and the approvals panel in `src/Popover.tsx`. `npm run ui` opens the settings window in a browser on made-up data from `src/bridge.ts`; add `?popover` to the address for the panel.
- `src-tauri/tauri.conf.json`: app name, bundle id, icons, and the updater's public key. Don't change the key; every installed copy uses it to check updates.
- `src-tauri/tauri.release.conf.json`: only used by the release workflow, to build the signed update bundle
- `.github/workflows/release.yml` and `RELEASING.md`: how releases get built and published
- `assets/icon.png`: source icon. Its art reaches every edge; macOS rounds it. Regenerate the app icons with `npx tauri icon assets/icon.png -o src-tauri/icons`
- `assets/Headroom.icon`: the same art for macOS 26, which puts icons that only come the older way on a gray plate in notifications. After changing the art, copy it to `assets/Headroom.icon/Assets/art.png` and rebuild `src-tauri/icons/Assets.car` (needs Xcode): `xcrun actool assets/Headroom.icon --compile src-tauri/icons --app-icon Headroom --platform macosx --target-device mac --minimum-deployment-target 11.0 --output-partial-info-plist /tmp/partial.plist`

Don't commit `node_modules/`, `src-tauri/target/`, or `src-tauri/gen/`.
