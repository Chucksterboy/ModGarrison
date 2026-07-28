import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GameProfile } from "../shared/contracts";
import { DiscoverView } from "./App";

const profile: GameProfile = {
  id: "profile-mound",
  name: "The Mound: Omen of Cthulhu",
  gamePath: "D:/Steam/steamapps/common/The Mound",
  steamAppId: "2569760",
  engine: "unreal",
  loader: "ue4ss",
  setupStatus: "ready",
  setupWarnings: [],
  modsEnabled: true,
  createdAt: "2026-07-28T00:00:00Z",
  updatedAt: "2026-07-28T00:00:00Z"
};

function renderDiscover(
  onLoad: ReturnType<typeof vi.fn>,
  {
    hasLoaded = true,
    providerWarning = "",
    total = 0
  }: {
    hasLoaded?: boolean;
    providerWarning?: string;
    total?: number;
  } = {}
) {
  render(
    <DiscoverView
      hasLoaded={hasLoaded}
      installCompletionId={0}
      installedMods={[]}
      installingModId=""
      isLoadingInstalledMods={false}
      isLoading={false}
      mods={[]}
      modPresentations={{}}
      removingModId=""
      total={total}
      providerWarning={providerWarning}
      profiles={[profile]}
      selectedProfileId={profile.id}
      onInstall={() => undefined}
      onLoadFiles={async () => []}
      onNeedsAuth={() => undefined}
      onOpenPage={() => undefined}
      onLoad={onLoad}
      onRemoveInstalledMod={() => undefined}
      onSelectProfile={() => undefined}
    />
  );
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Discover refresh behavior", () => {
  it("forces a provider refresh only from the explicit load button", () => {
    const onLoad = vi.fn().mockResolvedValue(true);
    renderDiscover(onLoad, { hasLoaded: false });

    fireEvent.click(screen.getByRole("button", { name: "Load Online Mods" }));

    expect(onLoad).toHaveBeenCalledWith(1, "downloads", "", true);
  });

  it("keeps sorting, pagination, and search on the normal cached path", async () => {
    vi.useFakeTimers();
    const onLoad = vi.fn().mockResolvedValue(true);
    renderDiscover(onLoad, { total: 21 });

    fireEvent.click(screen.getByRole("button", { name: "Newest" }));
    expect(onLoad).toHaveBeenCalledWith(1, "newest", "", false);

    onLoad.mockClear();
    fireEvent.click(screen.getAllByRole("button", { name: "Next" })[0]);
    expect(onLoad).toHaveBeenCalledWith(2, "downloads", "", false);

    onLoad.mockClear();
    fireEvent.change(screen.getByPlaceholderText("Search online mods"), {
      target: { value: "mound" }
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      await Promise.resolve();
    });
    expect(onLoad).toHaveBeenCalledWith(1, "downloads", "mound", false);
  });

  it("keeps the provider warning visible when discovery has no results", () => {
    const warning =
      "Nexus Mods catalogue could not be linked automatically for The Mound.";
    renderDiscover(vi.fn().mockResolvedValue(true), { providerWarning: warning });

    expect(screen.getByText(warning)).toBeTruthy();
    expect(
      screen.queryByText("No online provider results were found for this profile.")
    ).toBeNull();
  });
});
