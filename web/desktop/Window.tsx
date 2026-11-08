import { Component, Suspense, useRef, type Dispatch, type PointerEvent, type ReactNode } from "react";
import { motion } from "motion/react";
import { Maximize2, Minimize2, Minus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import i18n from "../i18n";
import type { Bounds, Size, Win, WmAction } from "./windows";

// Below Tailwind's `sm` breakpoint windows are full screen and can't be dragged (CSS in className below).
const isPhone = () => matchMedia("(max-width: 639px)").matches;

type Props = {
  win: Win;
  title: string;
  focused: boolean;
  viewport: Size;
  dispatch: Dispatch<WmAction>;
  children: ReactNode;
};

export function Window({ win, title, focused, viewport, dispatch, children }: Props) {
  const { t } = useTranslation();
  const drag = useRef<{ mode: "move" | "resize"; px: number; py: number; start: Bounds } | null>(null);
  const { id, bounds, maximized, minimized } = win;

  const begin = (mode: "move" | "resize") => (e: PointerEvent) => {
    if (e.button !== 0 || maximized || isPhone()) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { mode, px: e.clientX, py: e.clientY, start: bounds };
  };
  const move = (e: PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.px;
    const dy = e.clientY - d.py;
    const next =
      d.mode === "move"
        ? { ...d.start, x: d.start.x + dx, y: d.start.y + dy }
        : { ...d.start, w: d.start.w + dx, h: d.start.h + dy };
    dispatch({ type: "setBounds", id, bounds: next, viewport });
  };
  const end = () => (drag.current = null);
  const stop = (e: PointerEvent) => e.stopPropagation(); // title-bar buttons must not start a drag

  return (
    <motion.section
      role="dialog"
      aria-label={title}
      initial={{ opacity: 0, scale: 0.96 }}
      animate={
        minimized
          ? { opacity: 0, scale: 0.9, y: 40, transitionEnd: { visibility: "hidden" } }
          : { opacity: 1, scale: 1, y: 0, visibility: "visible" }
      }
      exit={{ opacity: 0, scale: 0.96 }}
      transition={{ duration: 0.15 }}
      onPointerDownCapture={() => focused || dispatch({ type: "focus", id })}
      style={{
        zIndex: win.z,
        ...(maximized ? { inset: 0 } : { left: bounds.x, top: bounds.y, width: bounds.w, height: bounds.h }),
      }}
      className={`absolute flex flex-col overflow-hidden rounded-box border bg-base-100 shadow-xl max-sm:inset-0! max-sm:h-auto! max-sm:w-auto! max-sm:rounded-none! ${
        maximized ? "rounded-none" : ""
      } ${focused ? "border-base-300" : "border-base-200 opacity-95"}`}
    >
      <header
        className={`flex h-9 shrink-0 cursor-default items-center gap-1 border-b border-base-200 px-2 select-none ${
          focused ? "bg-base-200" : "bg-base-100"
        }`}
        onPointerDown={begin("move")}
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={end}
        onDoubleClick={() => dispatch({ type: "toggleMaximize", id })}
      >
        <span className="flex-1 truncate px-1 text-sm font-medium">{title}</span>
        <button
          className="btn btn-ghost btn-xs btn-square"
          aria-label={t("desktop.window.minimize")}
          onPointerDown={stop}
          onClick={() => dispatch({ type: "minimize", id })}
        >
          <Minus size={14} />
        </button>
        <button
          className="btn btn-ghost btn-xs btn-square max-sm:hidden"
          aria-label={t(maximized ? "desktop.window.restore" : "desktop.window.maximize")}
          onPointerDown={stop}
          onClick={() => dispatch({ type: "toggleMaximize", id })}
        >
          {maximized ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
        </button>
        <button
          className="btn btn-ghost btn-xs btn-square hover:btn-error"
          aria-label={t("desktop.window.close")}
          onPointerDown={stop}
          onClick={() => dispatch({ type: "close", id })}
        >
          <X size={14} />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-auto">
        <AppErrorBoundary>
          <Suspense fallback={<span className="loading loading-spinner m-6" />}>{children}</Suspense>
        </AppErrorBoundary>
      </div>

      {!maximized && (
        // shortcut: resize from the bottom-right corner only; add edge handles if users ask for them.
        <div
          role="presentation"
          title={t("desktop.window.resize")}
          className="absolute right-0 bottom-0 size-4 cursor-se-resize max-sm:hidden"
          onPointerDown={begin("resize")}
          onPointerMove={move}
          onPointerUp={end}
          onPointerCancel={end}
        />
      )}
    </motion.section>
  );
}

/** A crashing app (or a failed query) only takes down its own window, not the desktop. */
class AppErrorBoundary extends Component<{ children: ReactNode }, { error?: Error }> {
  state: { error?: Error } = {};
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div role="alert" className="alert alert-error alert-soft m-4">
        <span>{this.state.error.message}</span>
        <button className="btn btn-sm" onClick={() => this.setState({ error: undefined })}>
          {i18n.t("desktop.window.retry")}
        </button>
      </div>
    );
  }
}
