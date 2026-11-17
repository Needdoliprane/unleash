import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../App";
import { buildSafeFallbackMarkdownAst, sanitizeHttpUrl } from "../ipc/tauriBridge";

describe("AC-F014-014 — Neutralisation des injections et absence de canari secret", () => {
  it("rend inertes les injections HTML, SVG, attributs onload/onerror et URLs javascript:/data:", () => {
    const hostileMarkdown = [
      "# Titre <script>window.__xss=1</script>",
      "",
      '<svg onload="alert(1)"><script>alert(2)</script></svg>',
      "",
      '<img src=x onerror="alert(3)">',
      "",
      "[Lien piege](javascript:alert(4))",
      "",
      "[Data piege](data:text/html,<script>alert(5)</script>)",
      "",
      "[Lien autorise](https://example.org/docs)",
    ].join("\n");

    const ast = buildSafeFallbackMarkdownAst(hostileMarkdown);
    expect(ast.blocked_raw_html_count).toBeGreaterThanOrEqual(2);
    expect(ast.blocked_url_count).toBe(2);

    const { container } = render(
      <App agentCount={10} eventTotalCount={10} diffLineCount={20} initialReportAst={ast} />,
    );

    // Aucun élément HTML exécutable injecté dans le DOM
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("svg[onload]")).toBeNull();
    expect(container.querySelector("img[onerror]")).toBeNull();

    // Aucun lien javascript: ou data: dans le DOM
    const anchors = Array.from(container.querySelectorAll("a"));
    for (const anchor of anchors) {
      const href = anchor.getAttribute("href") ?? "";
      expect(href.startsWith("https://") || href.startsWith("http://")).toBe(true);
    }

    const reportRegion = screen.getByTestId("markdown-ast-report");
    expect(reportRegion.textContent).toContain("Lien autorise");
    expect(reportRegion.textContent).toContain("[URL bloquée]");
    expect(sanitizeHttpUrl("javascript:alert(1)")).toBeNull();
    expect(sanitizeHttpUrl("data:text/html,hi")).toBeNull();
    expect(sanitizeHttpUrl("https://unleash.local/doc")).toBe("https://unleash.local/doc");
  });

  it("n'expose aucun secret canari dans le DOM, localStorage ou sessionStorage", () => {
    const canarySecret = "UNLEASH_CANARY_SECRET_9f3c7a12";
    const inspectStorage = (storage: Storage | undefined): void => {
      if (!storage || typeof storage.getItem !== "function") {
        return;
      }
      for (let i = 0; i < storage.length; i += 1) {
        const key = storage.key(i);
        if (key !== null) {
          expect(storage.getItem(key) ?? "").not.toContain(canarySecret);
        }
      }
    };

    const { container } = render(<App agentCount={10} eventTotalCount={10} diffLineCount={20} />);
    expect(container.textContent).not.toContain(canarySecret);
    for (const el of Array.from(container.querySelectorAll("*"))) {
      for (const attr of Array.from(el.attributes)) {
        expect(attr.value).not.toContain(canarySecret);
      }
    }

    inspectStorage(window.localStorage);
    inspectStorage(window.sessionStorage);
  });
});
