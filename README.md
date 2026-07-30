# Mod Garrison

[Download releases](https://github.com/Chucksterboy/ModGarrison/releases) |
[Privacy policy](PRIVACY.md) |
[Code signing policy](CODE_SIGNING.md) |
[Bundled asset licenses](ASSET_LICENSES.md) |
[MIT license](LICENSE)

Mod Garrison is a desktop mod manager prototype built around a practical universal model:

- One app and profile system.
- Per-game install adapters.
- Automatic Client, Hosted Server, and Dedicated Server route detection with a per-mod target selector when an archive contains real alternatives.
- Dependency planning before files are written.
- Automatic runtime and nested dependency downloads from official providers into the same verified game-root family as the parent mod.
- Transactional installs, updates, target switches, disable/remove operations, runtime repairs, and profile imports with crash recovery.
- Fail-closed archive planning for ambiguous routes, incomplete companion files, collisions, and missing or tampered recovery data.
- Tauri shell with a React frontend and Rust backend commands.
- Managed per-profile package copies so mods can be disabled and re-enabled even if the original download is removed.
- Data-driven game definitions for known game IDs, runtime overrides, and config roots.
- Startup update checks against GitHub Releases, with a sidebar indicator when a newer release is available.
- Profile import/export bundles for sharing a profile's managed mods and config files with another player.

The first implementation supports ZIP archives, 7Z archives, and normal folder imports for these install families:

- BepInEx / Thunderstore-style Unity mods.
- UE4SS / Unreal mod structures.
- REFramework / RE Engine scripts and plugins.
- Generic Unreal `.pak` style mods.
- Loose-file fallback with warnings.

## Runtime Providers

Mod Garrison can automatically resolve these loader/runtime dependencies:

- Thunderstore package dependencies from `manifest.json`.
- Valheim BepInEx from Thunderstore (`denikson/BepInExPack_Valheim`).
- BepInEx Mono x64 from official GitHub releases.
- BepInEx IL2CPP x64 from official BepInBuilds bleeding-edge artifacts.
- UE4SS from official GitHub releases.
- REFramework from official GitHub releases, using game-specific stable assets when a supported RE Engine game is detected and the generic nightly asset as a fallback.

Source platforms are handled according to their documented capabilities:

- Thunderstore supports direct discovery, download, installation, and package dependency metadata.
- Nexus Mods uses its official API, with the browser/NXM handoff when required by Nexus account and download rules.
- GameBanana supports direct discovery and archive downloads through its public API.
- mod.io supports direct downloads from officially available, free catalogues that permit read-only API-key access; user-authenticated or restricted catalogues are not bypassed.
- Steam Workshop lists only content already downloaded or subscribed through Steam. Steam remains responsible for acquiring and updating Workshop items.
- CurseForge/Overwolf remains subject to its provider-specific API and download rules and is not scraped.

### Runtime Registry Extensions

Bundled runtime definitions live in `src-tauri/src/runtime_definitions.json`. A local extension registry can add a runtime, or override a bundled runtime with the same `id`, without recompiling Mod Garrison:

```text
%APPDATA%\com.modgarrison.desktop\registry\runtime-definitions.json
```

The extension file uses the same JSON array shape as the bundled registry. Mod Garrison validates runtime IDs, provider capabilities, detection marker types, and every relative detection path before accepting the registry. An invalid extension blocks startup with a specific registry error instead of silently falling back to unsafe or partial behavior. New provider resolvers still require an audited native implementation and an entry in `src-tauri/src/dependency_provider_definitions.json`; registry data cannot turn an unsupported provider into an automatic downloader.

## Run

Mod Garrison now uses Tauri, so running from source requires Rust/Cargo plus the bundled Codex Node runtime or a local Node.js install.

```powershell
pnpm install
pnpm dev
```

The Desktop shortcut points at `Start-Mod-Garrison.cmd`. It starts a release build when one exists, otherwise it launches Tauri dev mode. If Rust is not installed, it will show a clear message instead of failing silently.

## Current State

This is a working foundation, not a finished public mod manager. It can create game profiles, detect game engines/loaders from a selected game folder, scan imported ZIP archives, detect common mod layouts, build an install plan, download supported runtime dependencies, discover supported provider catalogues, and install files with backups. Provider behavior remains limited to the official API, authentication, distribution, and download capabilities described above.

The React frontend lives in `src/renderer`. The Tauri/Rust backend lives in `src-tauri`.

The transfer tab exports a selected profile into a `.modgarrison-profile` bundle. Importing that bundle prompts for the local game folder, recreates the profile, restores bundled config files, and redeploys enabled mods from the managed package copies. Legacy `.uniloader-profile` bundles remain importable.

## Releases and Updates

Mod Garrison displays its current app version in the bottom-left rail. On startup, it checks the latest GitHub Release for `Chucksterboy/ModGarrison`. If a newer release exists, a pulsing download indicator appears above the health dot in the left rail.

The official Windows `.exe` installer and its SHA-256 checksum are published on
the [GitHub Releases page](https://github.com/Chucksterboy/ModGarrison/releases).

To publish an installer build, push a version tag:

```powershell
git tag v0.2.1
git push origin v0.2.1
```

The GitHub Actions release workflow validates the project, builds the single
Windows NSIS `.exe` installer, publishes the release, and uploads its SHA-256
checksum.

Official builds receive mod.io read-only application access from the
`MOD_GARRISON_MODIO_API_KEY` repository secret and the account-specific API path
from the `MOD_GARRISON_MODIO_API_BASE` repository variable. The API path should
be copied from [mod.io API Access](https://mod.io/me/access) and must use an
official `https://*.modapi.io/v1` endpoint. End users do not need to supply a
key, authenticate with mod.io, or accept an in-app consent prompt for this
read-only, non-user-authenticated catalogue flow; Settings links to mod.io's
Terms of Use and Privacy Policy.

## Future Game Support

Known-game rules live in `src-tauri/src/game_definitions.json`. Add future games there first:

- `executableNames` and `pathMarkers` identify the game.
- `bootstrapRuntimes` declares loaders Mod Garrison should install after profile creation.
- When `bootstrapRuntimes` is empty, profile creation and Refresh inspect the
  linked provider catalogue for a uniquely supported runtime. Known games only
  accept an inferred runtime when its adapter is explicitly listed in
  `supportedAdapters`.
- `runtimeDependencies` maps runtimes to official providers and release assets.
- `configRoots` adds game-specific config discovery paths.

Keep generic engine/loader detection in Rust, but prefer extending this manifest for game-specific behavior.

For games without a bundled provider domain, Discover automatically matches the
canonical Steam manifest title to the provider's game catalogue and caches the
verified mapping by Steam App ID. An explicit Refresh bypasses that cache and
resolves it again. This flow has no confirmation dialog or game chooser; if a
unique safe match cannot be established, discovery fails closed with a warning
instead of silently selecting a similarly named game.

## Safety Rules

- Never bypass a platform's login, API rules, rate limits, DRM, or anti-cheat.
- Prefer game-specific adapters over blind file-copy installs.
- Back up overwritten files before deploying into a game directory.
- Resolve dependencies per game profile, while caching downloads globally.
- Store imported mod sources in Mod Garrison's profile package library before deployment.

## Privacy

Mod Garrison is local-first and does not include project-operated telemetry,
analytics, advertising, accounts, or cloud synchronization. It does connect to
documented third-party services for updates, mod discovery, artwork, and
runtime downloads. See the [Privacy Policy](PRIVACY.md) for the complete data
and network disclosure.

## Code Signing Policy

Official Windows releases follow the project's
[Code signing policy](CODE_SIGNING.md).

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

## License

Mod Garrison is available under the [MIT License](LICENSE). Third-party
dependencies retain their respective licenses. Game artwork, provider content,
and mods fetched at runtime are not redistributed under the Mod Garrison license.
See [Bundled Asset Licenses](ASSET_LICENSES.md) for the provenance and license
of the notification sounds and application artwork included in release builds.
