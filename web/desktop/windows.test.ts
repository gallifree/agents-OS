import { describe, expect, it } from "vitest";
import { clamp, focusedId, initialWm, MIN_SIZE, wmReducer, type WmAction } from "./windows";

const viewport = { w: 1280, h: 720 };
const open = (appId: string): WmAction => ({ type: "open", appId, size: { w: 400, h: 300 }, viewport });
const run = (...actions: WmAction[]) => actions.reduce(wmReducer, initialWm);

describe("wmReducer", () => {
  it("opens windows on top, cascaded", () => {
    const s = run(open("a"), open("b"));
    expect(s.windows.map((w) => w.id)).toEqual(["a", "b"]);
    expect(focusedId(s)).toBe("b");
    expect(s.windows[1].bounds.x).toBeGreaterThan(s.windows[0].bounds.x);
  });

  it("re-opening a running app restores and focuses it instead of duplicating", () => {
    const s = run(open("a"), open("b"), { type: "minimize", id: "a" }, open("a"));
    expect(s.windows).toHaveLength(2);
    expect(s.windows[0].minimized).toBe(false);
    expect(focusedId(s)).toBe("a");
  });

  it("focus raises, minimize hands focus to the next window, close removes", () => {
    let s = run(open("a"), open("b"), { type: "focus", id: "a" });
    expect(focusedId(s)).toBe("a");
    s = wmReducer(s, { type: "minimize", id: "a" });
    expect(focusedId(s)).toBe("b");
    s = wmReducer(s, { type: "close", id: "b" });
    expect(focusedId(s)).toBeUndefined();
    expect(s.windows.map((w) => w.id)).toEqual(["a"]);
  });

  it("toggles maximize and focuses the window", () => {
    let s = run(open("a"), open("b"), { type: "toggleMaximize", id: "a" });
    expect(s.windows[0].maximized).toBe(true);
    expect(focusedId(s)).toBe("a");
    s = wmReducer(s, { type: "toggleMaximize", id: "a" });
    expect(s.windows[0].maximized).toBe(false);
  });

  it("clamps bounds so the title bar stays reachable and size stays usable", () => {
    const s = run(open("a"), { type: "setBounds", id: "a", bounds: { x: -5000, y: -50, w: 10, h: 10 }, viewport });
    const b = s.windows[0].bounds;
    expect(b.y).toBe(0);
    expect(b.w).toBe(MIN_SIZE.w);
    expect(b.h).toBe(MIN_SIZE.h);
    expect(b.x + b.w).toBeGreaterThan(0);
    expect(clamp({ x: 99999, y: 99999, w: 400, h: 300 }, viewport).x).toBeLessThan(viewport.w);
  });
});
