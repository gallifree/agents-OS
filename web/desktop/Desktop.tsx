import "../i18n";
import { useEffect, useReducer, useRef, useState } from "react";
import { AnimatePresence, MotionConfig } from "motion/react";
import { useTranslation } from "react-i18next";
import { RelayEnvironmentProvider } from "react-relay";
import { environment } from "../relay/environment";
import { appById } from "./apps";
import { Dock } from "./Dock";
import { applyTheme, savedTheme } from "./prefs";
import { TopBar } from "./TopBar";
import { Window } from "./Window";
import { focusedId, initialWm, wmReducer, type Size } from "./windows";

applyTheme(savedTheme());

export function Desktop() {
  const { t } = useTranslation();
  const [wm, dispatch] = useReducer(wmReducer, initialWm);
  const area = useRef<HTMLDivElement>(null);
  // shortcut: windows are clamped to the area on drag/resize only, not when the browser shrinks; reclamp on resize if it bites.
  const [viewport, setViewport] = useState<Size>({ w: innerWidth, h: innerHeight });

  useEffect(() => {
    const observer = new ResizeObserver(([entry]) =>
      setViewport({ w: entry.contentRect.width, h: entry.contentRect.height }),
    );
    observer.observe(area.current!);
    return () => observer.disconnect();
  }, []);

  const focused = focusedId(wm);
  const focusedApp = focused ? appById(wm.windows.find((w) => w.id === focused)!.appId) : undefined;

  const activate = (appId: string) => {
    if (focused === appId) dispatch({ type: "minimize", id: appId });
    else dispatch({ type: "open", appId, size: appById(appId)!.defaultSize, viewport });
  };

  return (
    <RelayEnvironmentProvider environment={environment}>
      {/* reducedMotion="user": animations are skipped when the OS asks for reduced motion. */}
      <MotionConfig reducedMotion="user">
        <div className="fixed inset-0 flex flex-col bg-linear-to-br from-base-300 via-base-200 to-base-300 text-base-content">
          <TopBar title={focusedApp && t(focusedApp.titleKey)} />
          <div ref={area} className="relative flex-1 overflow-hidden">
            <AnimatePresence>
              {wm.windows.map((win) => {
                const app = appById(win.appId)!;
                return (
                  <Window
                    key={win.id}
                    win={win}
                    title={t(app.titleKey)}
                    focused={win.id === focused}
                    viewport={viewport}
                    dispatch={dispatch}
                  >
                    <app.component />
                  </Window>
                );
              })}
            </AnimatePresence>
          </div>
          <Dock wm={wm} focused={focused} onActivate={activate} />
        </div>
      </MotionConfig>
    </RelayEnvironmentProvider>
  );
}
