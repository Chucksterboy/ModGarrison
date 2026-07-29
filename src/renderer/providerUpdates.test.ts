import { describe, expect, it } from "vitest";
import type { OnlineModFileOption } from "../shared/contracts";
import {
  installedRecordMatchesOnlineMod,
  preferredOnlineModFileId
} from "./App";

function nexusFile(
  id: string,
  name: string,
  primary = false
): OnlineModFileOption {
  return {
    id,
    name,
    primary,
    action: "direct"
  };
}

describe("preferredOnlineModFileId", () => {
  it("prefers the newest matching installed variant over the provider primary file", () => {
    const files = [
      nexusFile("server-new", "Dedicated Server"),
      nexusFile("client-new", "Client", true),
      nexusFile("server-old", "Dedicated Server")
    ];

    expect(
      preferredOnlineModFileId(files, {
        providerFileId: "server-old",
        providerVariant: " dedicated   server "
      })
    ).toBe("server-new");
  });

  it("falls back to the exact installed file and then the provider primary", () => {
    const files = [
      nexusFile("current", "Version 1"),
      nexusFile("primary", "Main File", true)
    ];

    expect(
      preferredOnlineModFileId(files, {
        providerFileId: "current",
        providerVariant: "No longer listed by this name"
      })
    ).toBe("current");
    expect(preferredOnlineModFileId(files)).toBe("primary");
  });
});

describe("installedRecordMatchesOnlineMod", () => {
  it("matches legacy Nexus and Thunderstore dependency identities", () => {
    expect(
      installedRecordMatchesOnlineMod(
        { dependencyString: "nexus:windrose/42#108" },
        { id: "nexus:windrose/42" }
      )
    ).toBe(true);
    expect(
      installedRecordMatchesOnlineMod(
        { dependencyString: "Author-FreeCrafting-1.2.3" },
        { id: "thunderstore:Author/FreeCrafting" }
      )
    ).toBe(true);
  });

  it("does not match a similarly prefixed provider package", () => {
    expect(
      installedRecordMatchesOnlineMod(
        { dependencyString: "nexus:windrose/420#108" },
        { id: "nexus:windrose/42" }
      )
    ).toBe(false);
    expect(
      installedRecordMatchesOnlineMod(
        { dependencyString: "Author-FreeCraftingPlus-1.2.3" },
        { id: "thunderstore:Author/FreeCrafting" }
      )
    ).toBe(false);
  });
});
