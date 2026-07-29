import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  GameProfile,
  InstalledModRecord,
  OnlineModFileOption,
  OnlineModRecord
} from "../shared/contracts";
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

const onlineMod: OnlineModRecord = {
  id: "nexus:windrose/42",
  provider: "nexus",
  providerLabel: "Nexus Mods",
  name: "Server Tools",
  owner: "Example Author",
  version: "2.0.0",
  description: "Client and server variants.",
  categories: [],
  downloads: 42,
  ratingScore: 5,
  dependencyCount: 0,
  installed: true,
  installSupported: true
};

const installedMod: InstalledModRecord = {
  id: "installed-server-tools",
  profileId: profile.id,
  archivePath: "D:/Mods/server-tools.zip",
  archiveName: "server-tools.zip",
  displayName: "Server Tools",
  packageId: onlineMod.id,
  providerFileId: "server-old",
  providerVariant: "Dedicated Server",
  adapterId: "ue4ss",
  summary: "Server tools",
  installedAt: "2026-07-28T00:00:00Z",
  filesWritten: [],
  backupsWritten: [],
  dependencies: [],
  configFiles: [],
  externallyManaged: false,
  enabled: true,
  lastStatus: "installed",
  installTargets: []
};

const onlineFiles: OnlineModFileOption[] = [
  {
    id: "server-new",
    name: "Dedicated Server",
    primary: false,
    action: "direct"
  },
  {
    id: "client-new",
    name: "Client",
    primary: true,
    action: "direct"
  },
  {
    id: "server-old",
    name: "Dedicated Server",
    primary: false,
    action: "direct"
  }
];

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

describe("Discover provider updates", () => {
  it("adopts the installed variant if installed state arrives after file loading", async () => {
    const onLoad = vi.fn().mockResolvedValue(true);
    const onLoadFiles = vi.fn().mockResolvedValue(onlineFiles);
    const sharedProps = {
      hasLoaded: true,
      installCompletionId: 0,
      installingModId: "",
      isLoadingInstalledMods: false,
      isLoading: false,
      mods: [onlineMod],
      modPresentations: {},
      removingModId: "",
      total: 1,
      providerWarning: "",
      profiles: [profile],
      selectedProfileId: profile.id,
      onInstall: () => undefined,
      onLoadFiles,
      onNeedsAuth: () => undefined,
      onOpenPage: () => undefined,
      onLoad,
      onRemoveInstalledMod: () => undefined,
      onSelectProfile: () => undefined
    };
    const rendered = render(
      <DiscoverView {...sharedProps} installedMods={[]} />
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "Show details for Server Tools"
      })
    );
    const client = (await screen.findByRole("radio", {
      name: /Client/
    })) as HTMLInputElement;
    expect(client.checked).toBe(true);

    rendered.rerender(
      <DiscoverView {...sharedProps} installedMods={[installedMod]} />
    );

    await waitFor(() => {
      const serverOptions = screen.getAllByRole("radio", {
        name: /Dedicated Server/
      }) as HTMLInputElement[];
      expect(serverOptions[0].checked).toBe(true);
      expect(serverOptions[1].checked).toBe(false);
    });
  });
});
