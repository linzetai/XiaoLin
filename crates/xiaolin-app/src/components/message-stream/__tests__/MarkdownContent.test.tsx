/**
 * @vitest-environment jsdom
 */

import { render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownContent } from "../MarkdownContent";

describe("MarkdownContent", () => {
  it("repairs common streamed markdown fence and heading spacing issues", async () => {
    const content = [
      "###轻微问题8.",
      "```typescriptvisibility?: ReasoningVisibility;",
      "```",
    ].join("\n");

    const { container } = render(<MarkdownContent content={content} />);

    await waitFor(() => {
      expect(container.querySelector("h3")?.textContent).toContain("轻微问题8.");
      expect(container.querySelector(".md-code-lang")?.textContent).toBe("typescript");
      expect(container.querySelector("code")?.textContent).toContain("visibility?: ReasoningVisibility;");
    });
    expect(container.textContent).not.toContain("typescriptvisibility?:");
  });
});
