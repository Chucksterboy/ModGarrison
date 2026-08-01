import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn()
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock
}));

import type {
  InstallRequest,
  ModActionResult,
  OnlineModFileOption,
  OnlineModRecord
} from "../shared/contracts";
import { desktopApi } from "./tauriApi";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("desktopApi.setModInstallTarget", () => {
  it("invokes the transactional install-target command with camel-case arguments", async () => {
    const result: ModActionResult = {
      profileId: "profile-one",
      installedModId: "mod-one",
      status: "installed",
      filesChanged: ["Client/Mods/example.dll"],
      warnings: []
    };
    invokeMock.mockResolvedValue(result);

    await expect(
      desktopApi.setModInstallTarget("profile-one", "mod-one", "client")
    ).resolves.toEqual(result);

    expect(invokeMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledWith("set_mod_install_target", {
      profileId: "profile-one",
      installedModId: "mod-one",
      installTargetId: "client"
    });
  });
});

describe("desktopApi.saveModioApiKey", () => {
  it("sends the application key override to the dedicated secure-storage command", async () => {
    const settings = {
      minimizeToTrayOnClose: false,
      nexusApiKeyConfigured: false,
      modioApiKeyConfigured: true
    };
    invokeMock.mockResolvedValue(settings);

    await expect(desktopApi.saveModioApiKey("developer-key")).resolves.toEqual(settings);

    expect(invokeMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledWith("save_modio_api_key", {
      apiKey: "developer-key"
    });
  });
});

describe("desktopApi install preparation", () => {
  it("binds preflight to the exact provider game and selected artifact", async () => {
    invokeMock.mockResolvedValue({
      dependencies: [],
      missingDependencies: [],
      confirmationRequired: false,
      installTargets: []
    });
    const mod = {
      id: "modio:55/901",
      provider: "modio",
      providerGameId: "55",
      version: "1.0.0"
    } as OnlineModRecord;
    const file = {
      id: "artifact-77",
      version: "1.1.0",
      primary: false,
      action: "direct"
    } as OnlineModFileOption;

    await desktopApi.preflightDiscoveredModInstall("profile-one", mod, file);

    expect(invokeMock).toHaveBeenCalledWith("preflight_discovered_mod_install", {
      profileId: "profile-one",
      provider: "modio",
      modId: "modio:55/901",
      version: "1.1.0",
      providerGameId: "55",
      selectedFileId: "artifact-77"
    });
  });

  it("passes the selected verified target with a manual install", async () => {
    invokeMock.mockResolvedValue({
      profileId: "profile-one",
      archivePath: "C:/managed/mod.zip",
      installedModId: "installed-one",
      installedAt: "now",
      filesWritten: [],
      backupsWritten: [],
      warnings: []
    });
    const request = {
      profileId: "profile-one",
      archivePath: "C:/Downloads/mod.zip",
      archiveName: "mod.zip",
      packageIdentity: {
        provider: "unknown",
        modTypes: [],
        dependencies: [],
        evidence: [],
        confidence: 1
      },
      plan: {
        adapterId: "unreal-pak",
        adapterName: "Unreal Pak",
        confidence: 1,
        summary: "fixture",
        mappings: [],
        dependencies: [],
        warnings: [],
        requiresConfirmation: false
      },
      installTargetId: "unreal-pak|r5/content/paks/~mods"
    } satisfies InstallRequest;

    await desktopApi.installArchive(request);

    expect(invokeMock).toHaveBeenCalledWith("install_archive", { request });
  });
});

describe("desktopApi discovery coordination", () => {
  it("passes the final warm-cache flag to discovery", async () => {
    const page = {
      items: [],
      total: 120,
      indexed: 20,
      complete: false,
      totalIsExact: false,
      page: 1,
      pageSize: 20,
      hasMore: false,
      providerWarnings: [],
      availableProviders: []
    };
    invokeMock.mockResolvedValue(page);

    await expect(
      desktopApi.discoverOnlineMods(
        "profile-one",
        1,
        20,
        "downloads",
        "",
        42,
        false,
        "nexus",
        true
      )
    ).resolves.toEqual(page);

    expect(invokeMock).toHaveBeenCalledWith("discover_online_mods", {
      profileId: "profile-one",
      page: 1,
      pageSize: 20,
      sort: "downloads",
      query: "",
      requestId: 42,
      forceRefresh: false,
      provider: "nexus",
      warmCache: true
    });
  });

  it("sends best-effort cancellation tokens to the backend", async () => {
    invokeMock.mockResolvedValue(undefined);

    await expect(
      desktopApi.cancelOnlineModDiscovery("profile-one", 43)
    ).resolves.toBeUndefined();

    expect(invokeMock).toHaveBeenCalledWith("cancel_online_mod_discovery", {
      profileId: "profile-one",
      requestId: 43
    });
  });
});
