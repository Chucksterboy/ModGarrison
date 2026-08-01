import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { InstallTargetOption } from "../shared/contracts";
import {
  InstallTargetChoices,
  InstallTargetDialog,
  InstallTargetPicker,
  LegacyInstallTargetChoices,
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

  it("does not invent a selection when the persisted target is missing or invalid", () => {
    expect(
      selectedInstallTargetOption({
        installTargets: [serverTarget, clientTarget]
      })
    ).toBeUndefined();

    expect(
      selectedInstallTargetOption({
        installTargets: [serverTarget, clientTarget],
        selectedInstallTargetId: "missing"
      })
    ).toBeUndefined();
  });
});

describe("InstallTargetDialog", () => {
  it("presents verified choices and confirms the selected real target", () => {
    const onConfirm = vi.fn();

    render(
      <InstallTargetDialog
        modName="Example Mod"
        motionClassName=""
        onCancel={() => undefined}
        onConfirm={onConfirm}
        targets={[clientTarget, serverTarget]}
      />
    );

    expect(
      screen.getByRole("dialog", { name: "Choose install target for Example Mod" })
    ).toBeTruthy();
    expect((screen.getByRole("radio", { name: /Client/ }) as HTMLInputElement).checked).toBe(
      true
    );

    fireEvent.click(screen.getByRole("radio", { name: /Server/ }));
    fireEvent.click(screen.getByRole("button", { name: "Install Here" }));

    expect(onConfirm).toHaveBeenCalledOnce();
    expect(onConfirm).toHaveBeenCalledWith(serverTarget.id);
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

  it("stays visible with a disabled placeholder when no target is selected", () => {
    const onChange = vi.fn();

    render(
      <InstallTargetPicker
        busy={false}
        modName="Example Mod"
        onChange={onChange}
        targets={[clientTarget, serverTarget]}
      />
    );

    const picker = screen.getByRole("combobox", {
      name: "Install target for Example Mod"
    }) as HTMLSelectElement;
    const placeholder = screen.getByRole("option", {
      name: "Choose target"
    }) as HTMLOptionElement;

    expect(picker.value).toBe("");
    expect(picker.disabled).toBe(false);
    expect(placeholder.disabled).toBe(true);

    fireEvent.change(picker, { target: { value: serverTarget.id } });
    expect(onChange).toHaveBeenCalledWith(serverTarget.id);
  });
});

describe("InstallTargetChoices", () => {
  it("renders radio-like choices and changes to an available target", () => {
    const onChange = vi.fn();

    render(
      <InstallTargetChoices
        busy={false}
        modName="Example Mod"
        onChange={onChange}
        selectedTarget={clientTarget}
        targets={[clientTarget, serverTarget]}
      />
    );

    const choices = screen.getByRole("radiogroup", {
      name: "Install targets for Example Mod"
    });
    const client = screen.getByRole("radio", { name: /Client/ });
    const server = screen.getByRole("radio", { name: /Server/ });

    expect(choices.getAttribute("aria-busy")).toBe("false");
    expect((client as HTMLInputElement).checked).toBe(true);
    expect((client as HTMLInputElement).disabled).toBe(false);
    expect((server as HTMLInputElement).checked).toBe(false);
    expect((server as HTMLInputElement).disabled).toBe(false);
    expect(screen.getByText("Installed")).toBeTruthy();
    expect(screen.getByText("Select")).toBeTruthy();

    fireEvent.click(server);
    expect(onChange).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenCalledWith(serverTarget.id);
  });

  it("shows the multi-location note and recommended choice without inventing a selection", () => {
    render(
      <InstallTargetChoices
        busy={false}
        modName="Example Mod"
        onChange={() => undefined}
        targets={[clientTarget, serverTarget]}
      />
    );

    expect(
      screen.getByText(
        "This mod is in multiple locations. Choose one target to remove the other managed copies."
      )
    ).toBeTruthy();
    expect(screen.getAllByRole("radio").every((choice) =>
      !(choice as HTMLInputElement).checked
    )).toBe(true);
    expect(screen.getByText("Recommended")).toBeTruthy();
    expect(screen.queryByText("Installed")).toBeNull();
  });

  it("locks every choice while this or another target change is pending", () => {
    const { rerender } = render(
      <InstallTargetChoices
        busy
        modName="Example Mod"
        onChange={() => undefined}
        targets={[clientTarget, serverTarget]}
      />
    );

    expect(screen.getAllByRole("radio").every((choice) =>
      (choice as HTMLInputElement).disabled
    )).toBe(true);
    expect(
      screen.getByRole("radiogroup").getAttribute("aria-busy")
    ).toBe("true");

    rerender(
      <InstallTargetChoices
        busy={false}
        disabled
        modName="Example Mod"
        onChange={() => undefined}
        targets={[clientTarget, serverTarget]}
      />
    );

    expect(screen.getAllByRole("radio").every((choice) =>
      (choice as HTMLInputElement).disabled
    )).toBe(true);
    expect(
      screen.getByRole("radiogroup").getAttribute("aria-busy")
    ).toBe("false");
  });
});

describe("LegacyInstallTargetChoices", () => {
  it("shows every legacy copy as installed without claiming one is primary", () => {
    render(
      <LegacyInstallTargetChoices
        targets={[
          { id: "game", label: "Game Mods", path: "Content/Paks/~mods" },
          {
            id: "server",
            label: "Local Server Mods",
            path: "R5/Builds/WindowsServer/R5/Content/Paks/~mods"
          }
        ]}
      />
    );

    expect(
      screen.getByText(
        "This legacy install is in multiple locations. Target switching needs an automatic upgrade or reinstall."
      )
    ).toBeTruthy();
    expect(screen.getAllByText("Installed")).toHaveLength(2);
    expect(screen.queryByText("Primary")).toBeNull();
    expect(screen.queryByRole("radio")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
