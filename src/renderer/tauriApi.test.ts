import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn()
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock
}));

import type { ModActionResult } from "../shared/contracts";
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
