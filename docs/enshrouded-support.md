# Enshrouded installation research and support contract

Reviewed 26 October 2026 against the mod authors' Nexus pages, current official
loader releases, and maintainer documentation. `G` below is the directory
containing `enshrouded.exe`; `S` contains `enshrouded_server.exe`. Paths are
relative to those roots unless a save location is stated.

This matrix records **25 distinct Nexus entries**. A deployment-layout test is
not an in-game compatibility test. The author instructions span different game
and mod versions; preserve the selected download's full companion tree and
verify its current requirements. Source pages are evidence, never executable
installation instructions supplied to the application at runtime.

ReShade is excluded from this work and remains manual, as requested by the user.

## Support in v0.13.0

The managed installation path recognizes complete EML mod folders, native
Shroudtopia factory plugins, and supported game-root proxy packages. It
preserves companion assets, automatically provisions the supported official
loaders, and tracks shared ownership through install, update, enable, disable,
and removal. Ambiguous layouts, unidentified loaders, incompatible forwarding
choices, and changed recovery data stop the operation before unsafe changes.

EML patch data is captured before managed installation and restored for
disable, clean launch, and removal. Automatically provisioned loaders are
retired only when unused. Native chaining prevents file collisions; it does
not establish that two mods' game hooks are compatible. Save imports,
standalone tools, custom loader builds, and hardware-specific dependencies
are not general automatic-install guarantees.

Max Stack's cogwheel exposes `enabled`, `maxStackSize`, `stackMultiplier`,
and `patchStackablesOnly`. Its `items.txt` panel edits existing overrides and
adds `ItemDebugName=stackSize` entries, with whole-number stack sizes from
1 to 2147483647. Saving an EML setting invalidates the patch cache so the next game
startup repatches even when two edits share the same timestamp second.

## Research matrix

| Nexus entry | Required layout / behavior | Integration requirement |
| --- | --- | --- |
| [Max Stack 111](https://www.nexusmods.com/enshrouded/mods/111) | Archive uses backslash member paths: `max-stack/mod.json`, `src/mod.lua`, `config.txt`, `items.txt`. Deploy the complete tree to `G/mods/max-stack/` with EML at root. | Cogwheel editing covers all four main settings plus existing and new item overrides. Preserve comments and settings on updates; repatch at the next game startup after a save. |
| [Creative Mode 146](https://www.nexusmods.com/enshrouded/mods/146) | Client `dinput8.dll` + `mods/creative/`; server download `powrprof.dll` + `mods/creative/`. Current native payload is `mods/creative/creative.dll` with Flame ownership metadata. | Keep native payload and settings together. Use its native load path rather than treating the manifest as an EML or Shroudtopia plugin. Participating clients also need the mod on dedicated servers. |
| [ShroudForge 147](https://www.nexusmods.com/enshrouded/mods/147) | Complete loader package beside executable; additional packages in `mods/<folder>/`. | Loader-specific manifest/configuration contract; arbitrary native DLLs are not EML scripts. |
| [Mod Menu 152](https://www.nexusmods.com/enshrouded/mods/152) | Root `POWRPROF.dll`, `aldrich-modmenu/aldrich_modmenu_layer.dll` and `aldrich-modmenu/aldrich-modmenu.ini`. | Root companion folder and proxy ownership. |
| [ModLoader Proxy 162](https://www.nexusmods.com/enshrouded/mods/162) | Rename official `modloader-proxy.dll` to client `dinput8.dll` or server `powrprof.dll`; root `chainloader.ini`, DLL paths under `[Load]`. | One canonical shared runtime/config; preserve manual entries. `ForwardTo` permits one input-export target, not arbitrary compatibility between mods. |
| [Minimap 97](https://www.nexusmods.com/enshrouded/mods/97) | `mods/minimap_mod/` containing manifest, DLL, image and binary assets. Requires Shroudtopia; root `shroudtopia.json` has module settings. | Detect plugin factory and keep the complete tree. Legacy release compatibility requires review. |
| [Miscellaneous Tweaks EML 67](https://www.nexusmods.com/enshrouded/mods/67) | `mods/<MOD_NAME>/mod.json` and scripts; EML root DLL. Some dedicated-server instructions use an EMM prelaunch patch step. | Provision the supported EML dependency; its automatic startup patching handles recognized patch-capability scripts. Selected mod and game versions still determine compatibility. |
| [Less Dense Shroud Fog 72](https://www.nexusmods.com/enshrouded/mods/72) | `mods/Less Dense Shroud Fog/src/mod.lua` and manifest; EML `dbghelp.dll` at root. | Client-only script/data mod with EML dependency. |
| [The Worst Map Mod 220](https://www.nexusmods.com/enshrouded/mods/220) | Root `dbgcore.dll` + `greeneye/unlock-map-markers/`; config `minimap_config.ini` inside that folder. | Shared Greeneye loader, client-only package. Nothing belongs on a server. |
| [Spawn The World 231](https://www.nexusmods.com/enshrouded/mods/231) | Root `dinput8.dll` + `SpawnTheWorld.ini`. | Independent native proxy and config companion; do not invent dedicated-server support. |
| [Flat Mountaintop Save 81](https://www.nexusmods.com/enshrouded/mods/81) | Author's local-save method imports world and companion files into `%USERPROFILE%/Saved Games/Enshrouded`, not G. | Resolve local/cloud save location and supported save format. Never replace a personal world or character silently. |
| [EnshroudedVR 172](https://www.nexusmods.com/enshrouded/mods/172) | Root `dinput8.dll` + `enshrouded_vr/`; OpenXR runtime/headset required. Author specifies windowed display, DLSS Quality, disabling automatic render scale and starting the VR connection first. | Hardware and game-display requirements remain real; deployment cannot manufacture a headset or guarantee VR behavior. Multiplayer untested by author. |
| [Sprint Toggle 165](https://www.nexusmods.com/enshrouded/mods/165) | Root `dinput8.dll` + `ToggleSprint.ini`; keyboard-only. | Resolve root proxy conflict without overwriting another mod. |
| [Climb Anywhere 214](https://www.nexusmods.com/enshrouded/mods/214) | Current package: root `dinput8.dll` + `climb-anywhere.ini`. Dedicated package goes in S; all clients need appropriate mod. Author permits renamed `climb-anywhere.dll` in the proxy list. | Older `climb-anywhere/` folder layout is obsolete. Server and client configurations must agree. |
| [XHL Creative Mode Fix 182](https://www.nexusmods.com/enshrouded/mods/182) | Shared XHL root `dinput8.dll` + `mods/XHL-Creative-Mode-Fix/`; requires Creative Mode. Author documents `dinput8-chain.ini` `[Chain]` entries for other native mods. | Preserve shared loader ownership; an old Creative 2.x route is not the current 3.x layout. |
| [XHL Core 236](https://www.nexusmods.com/enshrouded/mods/236) | Shared prerequisite contains root `dinput8.dll`. Other native proxies can be renamed and listed under `[Chain]` in `dinput8-chain.ini`. | Keep until all dependent mods are removed. Dedicated-server support is explicitly unverified. |
| [ShroudFix 233](https://www.nexusmods.com/enshrouded/mods/233) | Standalone Studio application; extract all files to a separate folder outside G and run `Run.exe`. It installs EML and the selected visibility setup. | This archive is an application/installer, not a direct mod payload. Do not run downloaded tooling as an arbitrary package script. |
| [ShroudForge Unified Tweaks 185](https://www.nexusmods.com/enshrouded/mods/185) | Requires ShroudForge; extract into `G/mods` and configure through the loader UI. | Correct loader identity and module configuration. |
| [Shroudscaler 113](https://www.nexusmods.com/enshrouded/mods/113) | Its `winmm.dll` replaces Shroudtopia's proxy; retain framework DLL/config. Requires entire separate OptiScaler download at root, including `D3D12_Optiscaler/` and dependency DLLs. | Author-specific intentional replacement, not a generic override. Do not discard companion graphics libraries. |
| [M182 Admin Panel 168](https://www.nexusmods.com/enshrouded/mods/168) | Complete main package beside executable. Ships a modified EML `dbghelp.dll`; author says install it last and only on host. First world launch exports data. | Replacing this DLL with stock EML can break the mod. Initial game-generated export is not installation completion. |
| [Skill Search 193](https://www.nexusmods.com/enshrouded/mods/193) | `mods/skill-search/` plus maintained patched Shroudtopia at root. | Native factory/loader compatibility, full companion folder. |
| [Convenience Mod EML 112](https://www.nexusmods.com/enshrouded/mods/112) | `mods/ConvenienceMod/` + EML root `dbghelp.dll`; edit `src/mod.lua` for options. | Author disables every feature by default. Successful deployment must not be described as all gameplay tweaks being active. |
| [DLSS Frame Generation 141](https://www.nexusmods.com/enshrouded/mods/141) | Complete archive at root, `EnshroudedVKFG.ini`, `Streamline/` and native proxy; RTX 40-series additionally needs MFGUnlocker, RTX 50-series does not. DLSS must be enabled in game. | Hardware-conditional dependencies. Author paragraph spells `winm.dll` while other references say `winmm.dll`; inspect actual archive instead of inventing a corrected filename. |
| [Shroudtopia Shroudweave 216](https://www.nexusmods.com/enshrouded/mods/216) | Current maintained loader: root `winmm.dll` + `shroudtopia.dll`; preserve `shroudtopia.json`. | Pin the loader release, not the repository's unrelated latest release. Keep shared loader after one dependent is removed. |
| [Minimap and Weather Clock 143](https://www.nexusmods.com/enshrouded/mods/143) | Complete `mods/minimap_mod/`, maintained Shroudtopia, Visual C++ x64 runtime. | Distinct updated entry from legacy Minimap 97. Full assets and configuration are necessary. |

## Audited public loader artifacts

The following identities were verified through the official repository release
API. The Shroudweave and ModLoader Proxy ZIPs were inspected in memory without
loading their DLLs or modifying the game installation.

- [EML v0.2.0](https://github.com/Brabb3l/kfc-parser/releases/tag/v0.2.0): raw
  `dbghelp.dll`, SHA-256
  `038b7a1787026007a68c82a6adeb8e79c2e644f936d4cd00f8b0db206a2349fa`.
  This is a raw Windows x64 DLL, not a ZIP or an arbitrary executable installer.
- [Shroudweave v0.2.1-shroudweave.1](https://github.com/Aerox912/shroudtopia/releases/tag/v0.2.1-shroudweave.1):
  `Shroudweave-Shroudtopia-0.2.1-shroudweave.1.zip`, SHA-256
  `851327c76e8deed259fec817035200da48c3f1d5c892f26afcbd2dbdfd893e56`.
  The verified DLL hashes are
  `7a8fa4b339dfc20e6d800fbb01b9d298d57ee7f623e14c606367db206fd385c5`
  (`shroudtopia.dll`) and
  `75159793a140fa37d506d3096e863ee209bac6738d195a22153a444ce52da9c9`
  (`winmm.dll`). The archive contains an example config rather than a root
  config, explicitly to preserve existing settings.
- [ModLoader Proxy 1.1.0](https://github.com/Entrodor/modloader-proxy/releases/tag/1.1.0):
  `modloader-proxy-1.1.0-x64.zip`, SHA-256
  `f661c3ac659ad0a07048ef45c01d3afb74579f70071119fe894507442a36745b`.
  Its x64 DLL hash is
  `dca7bcdc33368419a5bdab7eb740f62008be003f84e50704bd71c6dbad440c75`.
  The ZIP contains a wrapper directory and declares supported bootstrap names
  `dbghelp.dll`, `dinput8.dll`, `dsound.dll`, `powrprof.dll`, and `version.dll`.

The old `Aerox912/shroudtopia-1.0-compatibility` source snapshot is distinct from
the current maintained Shroudweave release. Its README identifies its original
Nexus file as quarantined and current-build support as unverified. Do not use
its mirror to evade the file review or treat it as the current loader.

## Loader and configuration rules

- [Maintained Shroudtopia documentation](https://shroudweave.aero-x.dev/install)
  distinguishes native factory plugins from Flame ownership. Native DLLs must
  export `CreateModInstance`; a `.dll` extension or the presence of `mod.json`
  alone is not proof. A missing manifest ID falls back to the folder name.
- [Official EML usage](https://brabb3l.github.io/kfc-parser/eml/usage.html)
  recommends the proxy DLL method as an alternative to the CLI. EML v0.2.0
  [enables patching at startup](https://github.com/Brabb3l/kfc-parser/blob/v0.2.0/crates/dbghelp-proxy/src/init.rs#L54-L66),
  so supported patch mods do not require a separate `emm.exe` command.
  [Its cache tracks mod files recursively](https://github.com/Brabb3l/kfc-parser/blob/v0.2.0/crates/mod-loader-lua/src/cache/track.rs).
  Capture the original KFC descriptor/resources before the first managed patch
  mod, track its generated containers, and restore the original data during
  removal. Shared dependencies remain until their last managed dependent is
  removed. The baseline is tied to the game build and never restored over a
  newer executable.
- Keep native proxy DLLs beside the executable when renaming them. Moving them
  to a different directory may break module-relative companion paths.
- Shared chainloader changes must be journaled with the canonical loader
  record and recomputed on install, update, enable, disable and removal. Keep
  unrelated manual entries and user settings. `ForwardTo` has one target;
  replacing an existing incompatible forwarding choice is not automatic.
- Preserving all files is necessary but does not establish binary hook
  compatibility. Two mods can patch the same game function even when their
  files no longer collide.
- [Official save documentation](https://enshrouded.zendesk.com/hc/en-us/articles/17764283835677-Save-Game-Management)
  separates local saves, Steam Cloud storage and dedicated-server save folders.
  Those destinations are not interchangeable with the game root.

## Verification boundaries

Release validation includes 344 backend tests and 86 frontend tests. Isolated
fixtures cover routing, complete companion files, runtime provisioning,
shared-file ownership, configuration editing and preservation, rollback, and
enable/disable/remove behavior. The Max Stack archive was also inspected as
files against the author's instructions. This does not constitute a gameplay
test of all 25 entries or their combinations. Do not run a downloaded DLL,
installer, patcher or game merely to inspect an archive.

Release claims must distinguish successful deployment tests from a real launch
with a particular game build and mod version. Nexus account authorization,
Windows administrator prompts, hardware requirements, author-required choices
and incompatible mod binaries cannot be removed by an installation path rule.
