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
