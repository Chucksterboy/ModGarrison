import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { InstallTargetOption } from "../shared/contracts";
import {
  InstallTargetPicker,
  installTargetRefreshWarning,
  selectedInstallTargetOption,
  withSelectedInstallTarget
} from "./App";

const clientTarget: InstallTargetOption = {
  id: "client",
  label: "Client",
  relativePath: "Client/Mods",
  scope: "client",
  recommended: true
};

const serverTarget: InstallTargetOption = {
  id: "server",
  label: "Server",
  relativePath: "Server/Mods",
  scope: "server",
  recommended: false
};

afterEach(cleanup);

describe("selectedInstallTargetOption", () => {
  it("prefers the persisted selection over the recommended target", () => {
    expect(
      selectedInstallTargetOption({
        installTargets: [clientTarget, serverTarget],
        selectedInstallTargetId: serverTarget.id
      })
    ).toEqual(serverTarget);
  });

  it("falls back to the recommended target and then the first target", () => {
    expect(
      selectedInstallTargetOption({
        installTargets: [serverTarget, clientTarget]
      })
    ).toEqual(clientTarget);

    expect(
      selectedInstallTargetOption({
        installTargets: [
          serverTarget,
          { ...clientTarget, recommended: false }
        ]
      })
    ).toEqual(serverTarget);
  });
});

describe("committed install-target state", () => {
  it("optimistically applies the selected target without changing other mods", () => {
    const mods = [
      { id: "mod-one", selectedInstallTargetId: clientTarget.id },
      { id: "mod-two", selectedInstallTargetId: serverTarget.id }
    ];

    const next = withSelectedInstallTarget(
      mods,
      "mod-one",
      serverTarget.id
    );

    expect(next).not.toBe(mods);
    expect(next[0]).toEqual({
      id: "mod-one",
      selectedInstallTargetId: serverTarget.id
    });
    expect(next[1]).toBe(mods[1]);
  });

  it("turns a post-commit refresh failure into a warning instead of rejecting", async () => {
    const warning = await installTargetRefreshWarning(async () => {
      throw new Error("store temporarily unavailable");
    });

    expect(warning).toContain("The target changed");
    expect(warning).toContain("store temporarily unavailable");
    await expect(
      installTargetRefreshWarning(async () => undefined)
    ).resolves.toBeUndefined();
  });
});

describe("InstallTargetPicker", () => {
  it("renders the persisted selection and emits a target change", () => {
    const onChange = vi.fn();

    render(
      <InstallTargetPicker
        busy={false}
        modName="Example Mod"
        onChange={onChange}
        selectedTarget={serverTarget}
        targets={[clientTarget, serverTarget]}
      />
    );

    const picker = screen.getByRole("combobox", {
      name: "Install target for Example Mod"
    }) as HTMLSelectElement;
    expect(picker.value).toBe(serverTarget.id);

    fireEvent.change(picker, { target: { value: clientTarget.id } });

    expect(onChange).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenCalledWith(clientTarget.id);
  });

  it("disables interaction while the target is changing", () => {
    const { container } = render(
      <InstallTargetPicker
        busy
        modName="Example Mod"
        onChange={() => undefined}
        selectedTarget={clientTarget}
        targets={[clientTarget, serverTarget]}
      />
    );

    const picker = screen.getByRole("combobox", {
      name: "Install target for Example Mod"
    }) as HTMLSelectElement;
    expect(picker.disabled).toBe(true);
    expect(
      container.querySelector(".vault-mod-target-picker")?.getAttribute("aria-busy")
    ).toBe("true");
  });

  it("can be globally locked without announcing every picker as busy", () => {
    const { container } = render(
      <InstallTargetPicker
        busy={false}
        disabled
        modName="Other Mod"
        onChange={() => undefined}
        selectedTarget={clientTarget}
        targets={[clientTarget, serverTarget]}
      />
    );

    const picker = screen.getByRole("combobox", {
      name: "Install target for Other Mod"
    }) as HTMLSelectElement;
    expect(picker.disabled).toBe(true);
    expect(
      container.querySelector(".vault-mod-target-picker")?.getAttribute("aria-busy")
    ).toBe("false");
  });

  it("stays hidden when there is no alternate target", () => {
    render(
      <InstallTargetPicker
        busy={false}
        modName="Example Mod"
        onChange={() => undefined}
        selectedTarget={clientTarget}
        targets={[clientTarget]}
      />
    );

    expect(screen.queryByRole("combobox")).toBeNull();
  });
});
