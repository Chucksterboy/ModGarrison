# Privacy Policy

Effective date: July 24, 2026

Mod Garrison is a local-first desktop mod manager. It does not operate a
Mod Garrison account service, advertising service, analytics service, telemetry
service, or cloud synchronization service.

This policy describes what the application stores locally, when it connects to
third-party services, and what may be included when you explicitly export a
profile.

## Data Stored on Your Computer

Mod Garrison may store the following data in its application data directory:

- Game profiles, including profile IDs, game names, Steam application IDs,
  local game installation paths, detected engines and runtimes, and profile
  status.
- Installed-mod records, enabled or disabled state, deployment receipts,
  dependency information, file hashes, backups, and transaction records.
- Managed copies of imported mod packages so enabled mods can be restored
  without the original download.
- Application settings, provider cache data, Steam artwork caches, update
  metadata, and temporary download data.
- Configuration files selected for profile export or managed as part of an
  installed mod.

On Windows, this data is normally stored under:

```text
%APPDATA%\com.modgarrison.desktop
```

When an existing UniLoader installation is upgraded, Mod Garrison migrates its
legacy `%APPDATA%\com.uniloader.desktop` data and Windows Credential Manager
entry locally. Newer files are never replaced by conflicting legacy files.

Mod Garrison scans local Steam library folders and Steam application manifests
when you ask it to find installed games. This information remains local except
for the network requests described below.

## Nexus Mods API Key

If you provide a Nexus Mods personal API key, Mod Garrison stores it in Windows
Credential Manager using the operating system credential store. The key is
sent only to Nexus Mods API endpoints for authentication, account validation,
mod discovery, dependency lookup, and supported download operations.

The API key is not included in exported Mod Garrison profiles. You can remove it
from Mod Garrison's settings at any time.

## Network Connections

Mod Garrison connects to third-party services only to provide requested features
or a documented startup operation:

- **GitHub:** checks the Mod Garrison repository for a newer release at startup,
  downloads an update when you request it, and obtains supported runtime
  releases hosted on GitHub.
- **Nexus Mods:** discovers mods, reads supported mod and dependency metadata,
  validates your API key, and handles supported download links.
- **Thunderstore:** discovers packages and reads package, version, and
  dependency metadata; downloads packages that you choose to install.
- **Steam content delivery networks:** retrieves game artwork for locally
  detected Steam games. Profile and banner artwork is cached locally.
- **BepInEx build services and runtime provider URLs:** retrieves runtime
  metadata and runtime archives when a supported game or mod needs them.
- **External mod pages:** opens a page in your default browser when provider
  rules require you to confirm a download or when you choose to view a page.

These services receive ordinary connection information such as your IP
address, request headers, and the requested resource. Their own policies govern
their processing:

- [GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)
- [Nexus Mods Privacy Policy](https://help.nexusmods.com/article/20-privacy-policy)
- [Thunderstore Privacy Policy](https://pages.thunderstore.io/p/privacy-policy)
- [Valve Privacy Policy](https://store.steampowered.com/privacy_agreement/)

Mod Garrison does not sell personal data and does not send usage analytics to the
project maintainer.

## Profile Import and Export

Profile export is an explicit user action. A `.modgarrison-profile` bundle may
contain:

- Profile metadata, including a local game installation path.
- Managed copies of mods included in that profile.
- Detected configuration files and their contents.
- Mod state, dependency, and deployment metadata.

Review a bundle and its contents before sharing it. A bundle can reveal local
folder names, configuration choices, and third-party mod files. You are
responsible for ensuring you have permission to redistribute any included mod
files. Imported bundles are processed locally and are not uploaded by
Mod Garrison.

## Retention and Deletion

Local data remains on your computer until you remove it through Mod Garrison or
delete the application data directory. Removing a profile asks Mod Garrison to
remove files it manages, subject to its safety checks and backups. Uninstalling
the application may not remove application data, cached files, or files already
deployed into game folders.

To remove all locally stored Mod Garrison application data after closing the
application, delete:

```text
%APPDATA%\com.modgarrison.desktop
```

You may also remove the Nexus Mods credential through Mod Garrison settings or
Windows Credential Manager.

## Security

Mod Garrison uses operating-system credential storage for provider credentials,
validates supported downloads and archives, and records managed file changes
to support safe disable and removal operations. No software can guarantee
absolute security. Keep Mod Garrison, Windows, and your game installations up to
date, and install mods only from sources you trust.

## Changes and Contact

Material changes to this policy will be committed to the public repository.
Questions or privacy concerns can be filed through
[GitHub Issues](https://github.com/Chucksterboy/ModGarrison/issues).
