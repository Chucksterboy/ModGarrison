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
  OnlineModProviderOption,
  OnlineModRecord
} from "../shared/contracts";
import { DiscoverView, ProgressiveDiscoveryCoordinator } from "./App";

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

const secondProfile: GameProfile = {
  ...profile,
  id: "profile-windrose",
  name: "Windrose",
  gamePath: "D:/Steam/steamapps/common/Windrose"
};

const availableProviders: OnlineModProviderOption[] = [
  {
    id: "thunderstore",
    label: "Thunderstore",
    modCount: 12
  },
  {
    id: "nexus",
    label: "Nexus Mods",
    modCount: 3
  },
  {
    id: "gamebanana",
    label: "GameBanana",
    modCount: 8
  },
  {
    id: "steam-workshop",
    label: "Steam Workshop",
    modCount: 5
  },
  {
    id: "modio",
    label: "mod.io",
    modCount: 2
  }
];

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

const steamWorkshopMod: OnlineModRecord = {
  ...onlineMod,
  id: "steam-workshop:1234567890",
  provider: "steam-workshop",
  providerLabel: "Steam Workshop",
  providerGameId: profile.steamAppId,
  name: "Workshop Balance Patch",
  owner: "Workshop Author",
  version: "workshop",
  description: "A Steam-managed balance patch.",
  packageUrl: "steam://url/CommunityFilePage/1234567890",
  websiteUrl: "https://steamcommunity.com/sharedfiles/filedetails/?id=1234567890",
  installed: true,
  installSupported: false,
  installNote: "Steam manages this Workshop item."
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
    availableProviders: providerOptions = [],
    complete = true,
    hasLoaded = true,
    isWarming = false,
    providerWarning = "",
    total = 0,
    indexed = total,
    totalIsExact = true
  }: {
    availableProviders?: OnlineModProviderOption[];
    complete?: boolean;
    hasLoaded?: boolean;
    indexed?: number;
    isWarming?: boolean;
    providerWarning?: string;
    total?: number;
    totalIsExact?: boolean;
  } = {}
) {
  render(
    <DiscoverView
      availableProviders={providerOptions}
      complete={complete}
      hasLoaded={hasLoaded}
      indexed={indexed}
      installCompletionId={0}
      installedMods={[]}
      installingModId=""
      isLoadingInstalledMods={false}
      isLoading={false}
      isWarming={isWarming}
      mods={[]}
      modPresentations={{}}
      removingModId=""
      total={total}
      totalIsExact={totalIsExact}
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

describe("Progressive discovery coordination", () => {
  it("waits between warm batches and never overlaps them", async () => {
    vi.useFakeTimers();
    let resolveFirst!: (continueWarming: boolean) => void;
    const firstBatch = new Promise<boolean>((resolve) => {
      resolveFirst = resolve;
    });
    const warmBatch = vi
      .fn<() => Promise<boolean>>()
      .mockImplementationOnce(() => firstBatch)
      .mockResolvedValueOnce(false);
    const coordinator = new ProgressiveDiscoveryCoordinator(1500);
    const generation = coordinator.beginSession();

    expect(coordinator.nextRequestId()).toBe(1);
    expect(coordinator.nextRequestId()).toBe(2);
    coordinator.schedule(generation, warmBatch);

    await vi.advanceTimersByTimeAsync(1500);
    expect(warmBatch).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(6000);
    expect(warmBatch).toHaveBeenCalledTimes(1);

    resolveFirst(true);
    await Promise.resolve();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(1499);
    expect(warmBatch).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(warmBatch).toHaveBeenCalledTimes(2);
  });

  it("drops stale warm work and keeps the replacement session sequential", async () => {
    vi.useFakeTimers();
    let resolveStale!: (continueWarming: boolean) => void;
    const staleBatch = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          resolveStale = resolve;
        })
    );
    const replacementBatch = vi.fn().mockResolvedValue(false);
    const coordinator = new ProgressiveDiscoveryCoordinator(1500);
    const staleGeneration = coordinator.beginSession();
    coordinator.schedule(staleGeneration, staleBatch);

    await vi.advanceTimersByTimeAsync(1500);
    expect(staleBatch).toHaveBeenCalledOnce();

    const replacementGeneration = coordinator.beginSession();
    coordinator.schedule(replacementGeneration, replacementBatch);
    await vi.advanceTimersByTimeAsync(6000);
    expect(replacementBatch).not.toHaveBeenCalled();

    resolveStale(true);
    await Promise.resolve();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(1499);
    expect(replacementBatch).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(replacementBatch).toHaveBeenCalledOnce();
    expect(coordinator.isCurrent(staleGeneration)).toBe(false);
    expect(coordinator.isCurrent(replacementGeneration)).toBe(true);
  });
});

describe("Discover refresh behavior", () => {
  it("forces a provider refresh only from the explicit load button", () => {
    const onLoad = vi.fn().mockResolvedValue(true);
    renderDiscover(onLoad, { hasLoaded: false });

    fireEvent.click(screen.getByRole("button", { name: "Load Online Mods" }));

    expect(onLoad).toHaveBeenCalledWith(1, "downloads", "", false, undefined);
  });

  it("keeps sorting, pagination, and search on the normal cached path", async () => {
    vi.useFakeTimers();
    const onLoad = vi.fn().mockResolvedValue(true);
    renderDiscover(onLoad, { total: 21 });

    fireEvent.click(screen.getByRole("button", { name: "Newest" }));
    expect(onLoad).toHaveBeenCalledWith(1, "newest", "", false, undefined);

    onLoad.mockClear();
    fireEvent.click(screen.getAllByRole("button", { name: "Next" })[0]);
    expect(onLoad).toHaveBeenCalledWith(2, "downloads", "", false, undefined);

    onLoad.mockClear();
    fireEvent.change(screen.getByPlaceholderText("Search online mods"), {
      target: { value: "mound" }
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      await Promise.resolve();
    });
    expect(onLoad).toHaveBeenCalledWith(1, "downloads", "mound", false, undefined);
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

  it("shows bounded indexing progress without disabling foreground controls", () => {
    renderDiscover(vi.fn().mockResolvedValue(true), {
      availableProviders,
      complete: false,
      indexed: 20,
      isWarming: true,
      total: 2600,
      totalIsExact: false
    });

    expect(
      screen.getByTitle("20 indexed; approximately 2600 total online mods")
        .textContent
    ).toContain("20 / ~2.6K");
    expect(
      (screen.getByRole("button", { name: "Refresh Mods" }) as HTMLButtonElement)
        .disabled
    ).toBe(false);
    expect(
      (screen.getByLabelText("Choose provider") as HTMLSelectElement).disabled
    ).toBe(false);
    expect(screen.queryByRole("button", { name: "Next" })).toBeNull();
  });
});

describe("Discover provider updates", () => {
  it("adopts the installed variant if installed state arrives after file loading", async () => {
    const onLoad = vi.fn().mockResolvedValue(true);
    const onLoadFiles = vi.fn().mockResolvedValue(onlineFiles);
    const sharedProps = {
      availableProviders: [],
      complete: true,
      hasLoaded: true,
      indexed: 1,
      installCompletionId: 0,
      installingModId: "",
      isLoadingInstalledMods: false,
      isLoading: false,
      isWarming: false,
      mods: [onlineMod],
      modPresentations: {},
      removingModId: "",
      total: 1,
      totalIsExact: true,
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

describe("Discover provider selector", () => {
  it("stays hidden when no provider has a real mod", () => {
    renderDiscover(vi.fn().mockResolvedValue(true), {
      availableProviders: [
        {
          id: "thunderstore",
          label: "Thunderstore",
          modCount: 0
        }
      ]
    });

    expect(screen.queryByLabelText("Choose provider")).toBeNull();
  });

  it("shows only positive-count providers and an all-providers option", () => {
    renderDiscover(vi.fn().mockResolvedValue(true), {
      availableProviders: [
        ...availableProviders,
        {
          id: "thunderstore",
          label: "Duplicate Thunderstore",
          modCount: 99
        }
      ]
    });

    const selector = screen.getByLabelText("Choose provider") as HTMLSelectElement;
    const options = Array.from(selector.options).map((option) => ({
      label: option.textContent,
      value: option.value
    }));

    expect(options).toEqual([
      { label: "All Providers", value: "all" },
      { label: "Thunderstore", value: "thunderstore" },
      { label: "Nexus Mods", value: "nexus" },
      { label: "GameBanana", value: "gamebanana" },
      { label: "Steam Workshop", value: "steam-workshop" },
      { label: "mod.io", value: "modio" }
    ]);
  });

  it.each([
    ["GameBanana", "gamebanana"],
    ["Steam Workshop", "steam-workshop"],
    ["mod.io", "modio"]
  ] as const)("passes the %s provider id through discovery", async (_label, providerId) => {
    const onLoad = vi.fn().mockResolvedValue(true);
    renderDiscover(onLoad, { availableProviders });

    const selector = screen.getByLabelText("Choose provider") as HTMLSelectElement;
    fireEvent.change(selector, { target: { value: providerId } });

    await waitFor(() => {
      expect(onLoad).toHaveBeenCalledWith(
        1,
        "downloads",
        "",
        false,
        providerId
      );
      expect(selector.value).toBe(providerId);
    });
  });

  it("loads page one for the chosen provider and retains it for other discovery actions", async () => {
    vi.useFakeTimers();
    const onLoad = vi.fn().mockResolvedValue(true);
    renderDiscover(onLoad, {
      availableProviders,
      total: 21
    });

    const selector = screen.getByLabelText("Choose provider") as HTMLSelectElement;
    fireEvent.change(selector, { target: { value: "nexus" } });
    await act(async () => {
      await Promise.resolve();
    });
    expect(onLoad).toHaveBeenLastCalledWith(1, "downloads", "", false, "nexus");
    expect(selector.value).toBe("nexus");

    onLoad.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Newest" }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(onLoad).toHaveBeenLastCalledWith(1, "newest", "", false, "nexus");

    onLoad.mockClear();
    fireEvent.click(screen.getAllByRole("button", { name: "Next" })[0]);
    await act(async () => {
      await Promise.resolve();
    });
    expect(onLoad).toHaveBeenLastCalledWith(2, "newest", "", false, "nexus");

    onLoad.mockClear();
    fireEvent.change(screen.getByPlaceholderText("Search online mods"), {
      target: { value: "mound" }
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      await Promise.resolve();
    });
    expect(onLoad).toHaveBeenLastCalledWith(1, "newest", "mound", false, "nexus");

    onLoad.mockClear();
    const refreshButton = screen.getByRole("button", { name: "Refresh Mods" });
    const totalModsBox = screen.getByTitle("21 total online mods");
    expect(refreshButton.parentElement).toBe(totalModsBox.parentElement);
    expect(totalModsBox.nextElementSibling).toBe(refreshButton);
    fireEvent.click(refreshButton);
    expect(onLoad).toHaveBeenLastCalledWith(1, "newest", "mound", true, "nexus");
  });

  it("keeps the previous provider when loading a new choice fails", async () => {
    const onLoad = vi.fn().mockResolvedValue(false);
    renderDiscover(onLoad, { availableProviders });

    const selector = screen.getByLabelText("Choose provider") as HTMLSelectElement;
    fireEvent.change(selector, { target: { value: "nexus" } });
    await waitFor(() => {
      expect(onLoad).toHaveBeenCalledWith(1, "downloads", "", false, "nexus");
    });

    expect(selector.value).toBe("all");
  });

  it("resets to all providers when the selected provider stops being available", async () => {
    const onLoad = vi.fn().mockResolvedValue(true);
    const sharedProps = {
      complete: true,
      hasLoaded: true,
      indexed: 0,
      installCompletionId: 0,
      installedMods: [],
      installingModId: "",
      isLoadingInstalledMods: false,
      isLoading: false,
      isWarming: false,
      mods: [],
      modPresentations: {},
      removingModId: "",
      total: 0,
      totalIsExact: true,
      providerWarning: "",
      profiles: [profile],
      selectedProfileId: profile.id,
      onInstall: () => undefined,
      onLoadFiles: async () => [],
      onNeedsAuth: () => undefined,
      onOpenPage: () => undefined,
      onLoad,
      onRemoveInstalledMod: () => undefined,
      onSelectProfile: () => undefined
    };
    const rendered = render(
      <DiscoverView {...sharedProps} availableProviders={availableProviders} />
    );

    fireEvent.change(screen.getByLabelText("Choose provider"), {
      target: { value: "nexus" }
    });
    await waitFor(() => {
      expect((screen.getByLabelText("Choose provider") as HTMLSelectElement).value).toBe(
        "nexus"
      );
    });
    onLoad.mockClear();

    rendered.rerender(
      <DiscoverView
        {...sharedProps}
        availableProviders={[availableProviders[0]]}
      />
    );

    await waitFor(() => {
      expect((screen.getByLabelText("Choose provider") as HTMLSelectElement).value).toBe(
        "all"
      );
      expect(onLoad).toHaveBeenCalledWith(1, "downloads", "", false);
    });
  });

  it("resets provider selection when the profile changes", async () => {
    const onLoad = vi.fn().mockResolvedValue(true);
    const sharedProps = {
      availableProviders,
      complete: true,
      hasLoaded: true,
      indexed: 0,
      installCompletionId: 0,
      installedMods: [],
      installingModId: "",
      isLoadingInstalledMods: false,
      isLoading: false,
      isWarming: false,
      mods: [],
      modPresentations: {},
      removingModId: "",
      total: 0,
      totalIsExact: true,
      providerWarning: "",
      profiles: [profile, secondProfile],
      onInstall: () => undefined,
      onLoadFiles: async () => [],
      onNeedsAuth: () => undefined,
      onOpenPage: () => undefined,
      onLoad,
      onRemoveInstalledMod: () => undefined,
      onSelectProfile: () => undefined
    };
    const rendered = render(
      <DiscoverView {...sharedProps} selectedProfileId={profile.id} />
    );

    fireEvent.change(screen.getByLabelText("Choose provider"), {
      target: { value: "nexus" }
    });
    await waitFor(() => {
      expect((screen.getByLabelText("Choose provider") as HTMLSelectElement).value).toBe(
        "nexus"
      );
    });

    rendered.rerender(
      <DiscoverView {...sharedProps} selectedProfileId={secondProfile.id} />
    );

    await waitFor(() => {
      expect((screen.getByLabelText("Choose provider") as HTMLSelectElement).value).toBe(
        "all"
      );
    });
  });
});

describe("Steam Workshop discovery cards", () => {
  it("keeps download, update, and removal owned by Steam", async () => {
    const onInstall = vi.fn();
    const onLoadFiles = vi.fn().mockResolvedValue(onlineFiles);
    const onOpenPage = vi.fn();
    render(
      <DiscoverView
        availableProviders={[
          {
            id: "steam-workshop",
            label: "Steam Workshop",
            modCount: 1
          }
        ]}
        complete
        hasLoaded
        indexed={1}
        installCompletionId={0}
        installedMods={[]}
        installingModId=""
        isLoadingInstalledMods={false}
        isLoading={false}
        isWarming={false}
        mods={[steamWorkshopMod]}
        modPresentations={{}}
        removingModId=""
        total={1}
        totalIsExact
        providerWarning=""
        profiles={[profile]}
        selectedProfileId={profile.id}
        onInstall={onInstall}
        onLoadFiles={onLoadFiles}
        onNeedsAuth={() => undefined}
        onOpenPage={onOpenPage}
        onLoad={vi.fn().mockResolvedValue(true)}
        onRemoveInstalledMod={() => undefined}
        onSelectProfile={() => undefined}
      />
    );

    expect(screen.getByText("Steam-managed")).toBeTruthy();
    const summaryAction = screen.getByRole("button", {
      name: "Open in Steam Workshop Balance Patch"
    });
    fireEvent.click(summaryAction);
    expect(onOpenPage).toHaveBeenLastCalledWith(
      "steam://url/CommunityFilePage/1234567890"
    );
    expect(onInstall).not.toHaveBeenCalled();
    expect(onLoadFiles).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Show details for Workshop Balance Patch"
      })
    );

    expect(await screen.findByText("Steam owns this download")).toBeTruthy();
    expect(
      screen.getByText(
        "Steam manages this item's subscription, download, updates, and removal."
      )
    ).toBeTruthy();
    expect(onLoadFiles).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Open in Steam" }));
    expect(onOpenPage).toHaveBeenCalledTimes(2);
    expect(onOpenPage).toHaveBeenLastCalledWith(
      "steam://url/CommunityFilePage/1234567890"
    );
    expect(onInstall).not.toHaveBeenCalled();
  });
});
