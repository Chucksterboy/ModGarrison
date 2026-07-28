import type { GameProfile, GarrisonCoreHealth } from "../shared/contracts";

type ProfileHealth = Pick<GameProfile, "setupStatus">;

export type CoreHealthPhase = "checking" | "healthy" | "degraded" | "error";

export interface CoreHealthState {
  phase: CoreHealthPhase;
  label: string;
  detail: string;
}

export const checkingCoreHealth: CoreHealthState = {
  phase: "checking",
  label: "Checking core systems…",
  detail: "Testing Garrison’s core systems and local saves."
};

export function profileNeedsAttention(
  profile: ProfileHealth | null | undefined,
  folderConnected: boolean | null | undefined,
  warningCount: number
): boolean {
  if (!profile) {
    return false;
  }

  return (
    profile.setupStatus === "needs-action" ||
    profile.setupStatus === "failed" ||
    folderConnected === false ||
    warningCount > 0
  );
}

export function classifyCoreHealth(
  health: GarrisonCoreHealth,
  localStorageReady: boolean
): CoreHealthState {
  const requiredFailures = [
    !health.backendReady ? "The native backend did not respond." : "",
    !localStorageReady ? "Local interface storage is unavailable." : "",
    !health.storageReady ? "Garrison’s settings or profile storage could not be read." : "",
    !health.modIndexReady ? "The managed-mod index could not be read." : "",
    !health.activeProfileReady ? "The active profile is missing or invalid." : ""
  ].filter(Boolean);

  if (requiredFailures.length > 0) {
    return {
      phase: "error",
      label: "Core systems unavailable",
      detail: requiredFailures.join(" ")
    };
  }

  const warnings = [
    ...health.warnings,
    !health.loadOrderReady ? "The managed deployment index needs attention." : "",
    !health.gameExecutableFound ? "The selected game executable was not found." : "",
    !health.modsDirectoryReady ? "The selected profile’s managed mods directory is unavailable." : "",
    health.usingFallback ? "Garrison is using a fallback service." : "",
    health.lastJob?.status === "failed"
      ? health.lastJob.summary || "The latest managed job failed."
      : ""
  ].filter(Boolean);

  if (warnings.length > 0) {
    return {
      phase: "degraded",
      label: "Needs attention",
      detail: [...new Set(warnings)].join(" ")
    };
  }

  const indexedLabel =
    health.indexedModCount === 1 ? "1 managed mod is indexed" : `${health.indexedModCount} managed mods are indexed`;
  return {
    phase: "healthy",
    label: "All systems functional",
    detail: `Core services and local saves are ready; ${indexedLabel}.`
  };
}

export function failedCoreHealth(caughtError: unknown): CoreHealthState {
  const rawMessage = caughtError instanceof Error ? caughtError.message : String(caughtError);
  const message = rawMessage.trim();
  return {
    phase: "error",
    label: "Health check failed",
    detail: message
      ? `Garrison could not complete its core health check. ${message}`
      : "Garrison could not complete its core health check."
  };
}
