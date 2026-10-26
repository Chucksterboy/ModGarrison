import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ModConfigFile } from "../shared/contracts";
import { ConfigFilePanel } from "./App";

const items: ModConfigFile = {
  path: "E:\\Games\\Enshrouded\\mods\\max-stack\\items.txt",
  fileName: "items.txt",
  entries: [],
  rawPreview: "# Item overrides\r\n# ItemDebugName=stackSize\r\n"
};

afterEach(cleanup);

function fillOverride(itemId: string, stackSize: string) {
  fireEvent.change(screen.getByRole("textbox", { name: "Item ID" }), { target: { value: itemId } });
  fireEvent.change(screen.getByRole("spinbutton", { name: "Stack size" }), { target: { value: stackSize } });
  fireEvent.click(screen.getByRole("button", { name: "Add override" }));
}

describe("Max Stack item overrides", () => {
  it("shows shipped comments and adds the first owned override, clearing its draft after saving", async () => {
    const onSaveValue = vi.fn().mockResolvedValue(undefined);
    render(<ConfigFilePanel entries={[]} file={items} onSaveValue={onSaveValue} />);

    expect(screen.getByText(/ItemDebugName=stackSize/).textContent).toContain("# Item overrides");
    fillOverride("Material_Wood", "1500");
    expect(onSaveValue).toHaveBeenCalledWith(items, { key: "Material_Wood", value: "" }, "1500");
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Item ID" }) as HTMLInputElement).value).toBe(""));
    expect((screen.getByRole("spinbutton", { name: "Stack size" }) as HTMLInputElement).value).toBe("");
  });

  it.each([
    ["../invalid/item", "5"],
    ["a".repeat(129), "5"],
    ["Material_Wood", "0"],
    ["Material_Wood", "-1"],
    ["Material_Wood", "1.5"],
    ["Material_Wood", "2147483648"],
    ["Material_Wood", ""]
  ])("rejects invalid item ID or stack size (%s, %s) before saving", (itemId, stackSize) => {
    const onSaveValue = vi.fn();
    render(<ConfigFilePanel entries={[]} file={items} onSaveValue={onSaveValue} />);
    fillOverride(itemId, stackSize);
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(onSaveValue).not.toHaveBeenCalled();
  });

  it("prevents a duplicate even when a search has hidden the existing entry", () => {
    const onSaveValue = vi.fn();
    const file = { ...items, entries: [{ key: "Material_Wood", value: "500" }] };
    render(<ConfigFilePanel entries={[]} file={file} onSaveValue={onSaveValue} />);
    fillOverride("material_wood", "1000");
    expect(screen.getByRole("alert").textContent).toContain("already has an override");
    expect(onSaveValue).not.toHaveBeenCalled();
  });

  it("retains the draft and shows a backend failure so the user can retry", async () => {
    const onSaveValue = vi.fn().mockRejectedValue(new Error("Game must be closed"));
    render(<ConfigFilePanel entries={[]} file={items} onSaveValue={onSaveValue} />);
    fillOverride("Material_Wood", "500");
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Game must be closed"));
    expect((screen.getByRole("textbox", { name: "Item ID" }) as HTMLInputElement).value).toBe("Material_Wood");
    expect((screen.getByRole("spinbutton", { name: "Stack size" }) as HTMLInputElement).value).toBe("500");
  });

  it("does not offer Max Stack syntax for another mod's items.txt", () => {
    const file = { ...items, path: "E:/Games/Enshrouded/mods/another-mod/items.txt" };
    render(<ConfigFilePanel entries={[]} file={file} onSaveValue={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Add override" })).toBeNull();
    expect(screen.getByText(/ItemDebugName=stackSize/)).toBeTruthy();
  });
});
