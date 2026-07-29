import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { NoticeBanner } from "./App";

afterEach(cleanup);

describe("NoticeBanner", () => {
  it("announces success and warning notices politely", () => {
    render(
      <NoticeBanner
        notice={{
          detail: "The selected target is active.",
          kind: "success",
          motionId: 1,
          title: "Install target updated"
        }}
      />
    );

    const status = screen.getByRole("status");
    expect(status.getAttribute("aria-live")).toBe("polite");
    expect(status.getAttribute("aria-atomic")).toBe("true");
  });

  it("announces errors assertively", () => {
    render(
      <NoticeBanner
        notice={{
          detail: "The target could not be changed.",
          kind: "error",
          motionId: 2,
          title: "Target change failed"
        }}
      />
    );

    expect(screen.getByRole("alert").getAttribute("aria-live")).toBe(
      "assertive"
    );
  });
});
