import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { FeedbackBanners, NoticeBanner } from "./App";

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

describe("FeedbackBanners", () => {
  it("renders a matching structured error only once", () => {
    const detail = "The provider request could not be completed.";
    render(
      <FeedbackBanners
        error={detail}
        notice={{
          detail,
          kind: "error",
          motionId: 3,
          title: "Discovery failed"
        }}
      />
    );

    expect(screen.getAllByText(detail)).toHaveLength(1);
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("keeps an error-only fallback assertive and accessible", () => {
    render(<FeedbackBanners error="The backend did not respond." />);

    const alert = screen.getByRole("alert");
    expect(alert.getAttribute("aria-live")).toBe("assertive");
    expect(alert.getAttribute("aria-atomic")).toBe("true");
  });
});
