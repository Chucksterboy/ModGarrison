import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GarrisonCoreHealth } from "../shared/contracts";
import {
  probeLocalStorage,
  SystemHealthButton
} from "./SystemHealthButton";

const healthyCore: GarrisonCoreHealth = {
  backendReady: true,
  storageReady: true,
  modIndexReady: true,
  activeProfileReady: true,
  loadOrderReady: true,
  gameExecutableFound: true,
  modsDirectoryReady: true,
  usingFallback: false,
  indexedModCount: 2,
  warnings: []
};

function setVisibility(value: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value
  });
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  setVisibility("visible");
  window.localStorage.clear();
});

describe("SystemHealthButton", () => {
  it("waits for application storage bootstrap before its first probe", async () => {
    const probeCore = vi.fn().mockResolvedValue(healthyCore);
    const rendered = render(
      <SystemHealthButton probeCore={probeCore} ready={false} />
    );

    expect(probeCore).not.toHaveBeenCalled();
    rendered.rerender(
      <SystemHealthButton probeCore={probeCore} ready />
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(probeCore).toHaveBeenCalledTimes(1);
  });

  it("starts in checking and renders the exact Starfold ECG before becoming healthy", async () => {
    let resolveProbe: ((value: GarrisonCoreHealth) => void) | undefined;
    const probeCore = vi.fn(
      () =>
        new Promise<GarrisonCoreHealth>((resolve) => {
          resolveProbe = resolve;
        })
    );
    const { container } = render(
      <SystemHealthButton profileId="profile-one" probeCore={probeCore} />
    );

    expect(screen.getByText("Checking core systems…")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Check Mod Garrison system health" })
        .getAttribute("aria-busy")
    ).toBe("true");
    expect(container.querySelector(".status-ecg-baseline")?.getAttribute("d")).toBe(
      "M1 14H45"
    );
    expect(container.querySelector(".status-ecg-trace")?.getAttribute("d")).toBe(
      "M1 14H10L13 11L16 22L21 4L25 18L29 12L33 14H45"
    );

    await act(async () => {
      resolveProbe?.(healthyCore);
      await Promise.resolve();
    });

    expect(probeCore).toHaveBeenCalledWith("profile-one");
    expect(screen.getByText("All systems functional")).toBeTruthy();
  });

  it("shows degraded and error phases from probe results and rejection", async () => {
    const degradedProbe = vi
      .fn()
      .mockResolvedValue({ ...healthyCore, gameExecutableFound: false });
    const degraded = render(
      <SystemHealthButton probeCore={degradedProbe} />
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Needs attention")).toBeTruthy();
    expect(
      degraded.container.querySelector(".status-health--degraded")
    ).toBeTruthy();
    degraded.unmount();

    render(
      <SystemHealthButton
        probeCore={vi.fn().mockRejectedValue(new Error("backend offline"))}
      />
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Health check failed")).toBeTruthy();
  });

  it("checks on click and every minute only while the document is visible", async () => {
    vi.useFakeTimers();
    const probeCore = vi.fn().mockResolvedValue(healthyCore);
    render(
      <SystemHealthButton intervalMs={60_000} probeCore={probeCore} />
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(probeCore).toHaveBeenCalledTimes(1);

    fireEvent.click(
      screen.getByRole("button", { name: "Check Mod Garrison system health" })
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(probeCore).toHaveBeenCalledTimes(2);

    setVisibility("hidden");
    act(() => vi.advanceTimersByTime(60_000));
    expect(probeCore).toHaveBeenCalledTimes(2);

    setVisibility("visible");
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
      await Promise.resolve();
    });
    expect(probeCore).toHaveBeenCalledTimes(3);

    await act(async () => {
      vi.advanceTimersByTime(60_000);
      await Promise.resolve();
    });
    expect(probeCore).toHaveBeenCalledTimes(4);
  });

  it("prevents duplicate checks and ignores a late result after unmount", async () => {
    let resolveProbe: ((value: GarrisonCoreHealth) => void) | undefined;
    const probeCore = vi.fn(
      () =>
        new Promise<GarrisonCoreHealth>((resolve) => {
          resolveProbe = resolve;
        })
    );
    const rendered = render(
      <SystemHealthButton probeCore={probeCore} />
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Check Mod Garrison system health" })
    );
    expect(probeCore).toHaveBeenCalledTimes(1);
    rendered.unmount();

    await act(async () => {
      resolveProbe?.(healthyCore);
      await Promise.resolve();
    });
    expect(probeCore).toHaveBeenCalledTimes(1);
  });
});

describe("probeLocalStorage", () => {
  it("round-trips and removes its disposable value", () => {
    expect(probeLocalStorage()).toBe(true);
    expect(window.localStorage.length).toBe(0);
  });

  it("returns false when storage writes are blocked", () => {
    const storage = {
      setItem: () => {
        throw new Error("blocked");
      },
      getItem: vi.fn(),
      removeItem: vi.fn(),
      clear: vi.fn(),
      key: vi.fn(),
      length: 0
    } as unknown as Storage;

    expect(probeLocalStorage(storage)).toBe(false);
  });
});
