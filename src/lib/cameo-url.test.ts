import { describe, expect, it } from "vitest";
import { buildCameoUrl, cameoProtocolBase } from "./cameo-url";

describe("cameoProtocolBase", () => {
  it("uses Tauri's Windows custom protocol shape", () => {
    const base = cameoProtocolBase((path, protocol) => `http://${protocol}.localhost/${encodeURIComponent(path)}`);

    expect(base).toBe("http://cameo.localhost");
  });

  it("uses native custom protocol shape on WebKit-style platforms", () => {
    const base = cameoProtocolBase((path, protocol) => `${protocol}://localhost/${encodeURIComponent(path)}`);

    expect(base).toBe("cameo://localhost");
  });

  it("falls back to the native custom protocol when Tauri internals are unavailable", () => {
    const base = cameoProtocolBase(() => {
      throw new Error("missing Tauri internals");
    });

    expect(base).toBe("cameo://localhost");
  });
});

describe("buildCameoUrl", () => {
  it("keeps board id in the path and encodes each relative path segment", () => {
    const url = buildCameoUrl("board 1", "imports/cute tiger#1.png", "http://cameo.localhost");

    expect(url).toBe("http://cameo.localhost/board%201/imports/cute%20tiger%231.png");
  });

  it("normalizes Windows separators without encoding path slashes", () => {
    const url = buildCameoUrl("board-1", String.raw`renders\final image.png`, "cameo://localhost/");

    expect(url).toBe("cameo://localhost/board-1/renders/final%20image.png");
  });

  it("strips leading slashes from workspace-relative paths", () => {
    const url = buildCameoUrl("board-1", "/imports/a.png", "http://cameo.localhost");

    expect(url).toBe("http://cameo.localhost/board-1/imports/a.png");
  });
});
