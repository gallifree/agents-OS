// Window manager state: a pure reducer, rendered by Desktop.

export type Bounds = { x: number; y: number; w: number; h: number };
export type Size = { w: number; h: number };

export type Win = {
  id: string;
  appId: string;
  bounds: Bounds;
  z: number;
  minimized: boolean;
  maximized: boolean;
};

export type WmState = { windows: Win[]; nextZ: number };

export type WmAction =
  | { type: "open"; appId: string; size: Size; viewport: Size }
  | { type: "focus" | "close" | "minimize" | "toggleMaximize"; id: string }
  | { type: "setBounds"; id: string; bounds: Bounds; viewport: Size };

export const MIN_SIZE: Size = { w: 280, h: 180 };
// How much of a window must stay on screen so its title bar can always be grabbed.
const KEEP_VISIBLE = 80;

export const initialWm: WmState = { windows: [], nextZ: 1 };

export function clamp(b: Bounds, vp: Size): Bounds {
  const w = Math.max(MIN_SIZE.w, Math.min(b.w, vp.w));
  const h = Math.max(MIN_SIZE.h, Math.min(b.h, vp.h));
  return {
    w,
    h,
    x: Math.min(Math.max(b.x, KEEP_VISIBLE - w), vp.w - KEEP_VISIBLE),
    y: Math.min(Math.max(b.y, 0), vp.h - KEEP_VISIBLE),
  };
}

/** The window that has focus: the topmost one that isn't minimized. */
export function focusedId(s: WmState): string | undefined {
  let top: Win | undefined;
  for (const w of s.windows) if (!w.minimized && (!top || w.z > top.z)) top = w;
  return top?.id;
}

function raise(s: WmState, id: string, patch: Partial<Win> = {}): WmState {
  return {
    nextZ: s.nextZ + 1,
    windows: s.windows.map((w) => (w.id === id ? { ...w, ...patch, z: s.nextZ } : w)),
  };
}

export function wmReducer(s: WmState, a: WmAction): WmState {
  switch (a.type) {
    case "open": {
      // One window per app: opening a running app brings it back.
      const existing = s.windows.find((w) => w.appId === a.appId);
      if (existing) return raise(s, existing.id, { minimized: false });
      const offset = 32 + (s.windows.length % 8) * 28; // cascade new windows
      const bounds = clamp({ x: offset, y: offset, ...a.size }, a.viewport);
      const win: Win = { id: a.appId, appId: a.appId, bounds, z: s.nextZ, minimized: false, maximized: false };
      return { nextZ: s.nextZ + 1, windows: [...s.windows, win] };
    }
    case "focus":
      return raise(s, a.id, { minimized: false });
    case "close":
      return { ...s, windows: s.windows.filter((w) => w.id !== a.id) };
    case "minimize":
      return { ...s, windows: s.windows.map((w) => (w.id === a.id ? { ...w, minimized: true } : w)) };
    case "toggleMaximize": {
      const w = s.windows.find((w) => w.id === a.id);
      return w ? raise(s, a.id, { maximized: !w.maximized, minimized: false }) : s;
    }
    case "setBounds":
      return {
        ...s,
        windows: s.windows.map((w) => (w.id === a.id ? { ...w, bounds: clamp(a.bounds, a.viewport) } : w)),
      };
  }
}
