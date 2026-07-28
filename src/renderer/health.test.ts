import { describe, expect, it } from "vitest";
import type { GarrisonCoreHealth } from "../shared/contracts";
import {
  classifyCoreHealth,
  failedCoreHealth,
  profileNeedsAttention
} from "./health";

const healthyCore: GarrisonCoreHealth = {
  backendReady: true,
  storageReady: true,
  modIndexReady: true,
  activeProfileReady: true,
  loadOrderReady: true,
  gameExecutableFound: true,
  modsDirectoryReady: true,
  usingFallback: false,
  indexedModCount: 3,
  warnings: []
};

describe("profileNeedsAttention", () => {
  it("keeps a ready connected profile healthy", () => {
    expect(profileNeedsAttention({ setupStatus: "ready" }, true, 0)).toBe(false);
  });

  it.each([
    [{ setupStatus: "needs-action" } as const, true, 0],
    [{ setupStatus: "failed" } as const, true, 0],
    [{ setupStatus: "ready" } as const, false, 0],
    [{ setupStatus: "ready" } as const, true, 1]
  ])("flags actionable profile state", (profile, connected, warnings) => {
    expect(profileNeedsAttention(profile, connected, warnings)).toBe(true);
  });

  it("does not invent an error when no profile is selected", () => {
    expect(profileNeedsAttention(null, false, 3)).toBe(false);
  });
});

describe("classifyCoreHealth", () => {
  it("reports a healthy native core and local-storage round trip", () => {
    expect(classifyCoreHealth(healthyCore, true)).toEqual({
      phase: "healthy",
      label: "All systems functional",
      detail: "Core services and local saves are ready; 3 managed mods are indexed."
    });
  });

  it("treats required storage and index failures as errors", () => {
    const result = classifyCoreHealth(
      { ...healthyCore, storageReady: false, modIndexReady: false },
      true
    );

    expect(result.phase).toBe("error");
    expect(result.detail).toContain("settings or profile storage");
    expect(result.detail).toContain("managed-mod index");
  });

  it("treats unavailable browser storage as an error", () => {
    expect(classifyCoreHealth(healthyCore, false).phase).toBe("error");
  });

  it("treats optional game-path and latest-job warnings as degraded", () => {
    const result = classifyCoreHealth(
      {
        ...healthyCore,
        gameExecutableFound: false,
        lastJob: {
          kind: "install",
          status: "failed",
          summary: "The latest install did not finish."
        }
      },
      true
    );

    expect(result.phase).toBe("degraded");
    expect(result.detail).toContain("game executable");
    expect(result.detail).toContain("latest install");
  });

  it("maps invoke rejection to the error phase", () => {
    expect(failedCoreHealth(new Error("native channel closed"))).toEqual({
      phase: "error",
      label: "Health check failed",
      detail:
        "Garrison could not complete its core health check. native channel closed"
    });
  });
});
