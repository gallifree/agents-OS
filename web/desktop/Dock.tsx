import { useTranslation } from "react-i18next";
import type { AppDef } from "./apps";
import type { WmState } from "./windows";

type Props = {
  apps: AppDef[];
  wm: WmState;
  focused: string | undefined;
  onActivate: (appId: string) => void;
};

export function Dock({ apps, wm, focused, onActivate }: Props) {
  const { t } = useTranslation();
  return (
    <nav
      aria-label={t("desktop.dock")}
      className="mx-auto my-2 flex gap-2 rounded-box border border-base-300 bg-base-100/80 p-2 shadow-lg backdrop-blur"
    >
      {apps.map((app) => {
        const running = wm.windows.some((w) => w.appId === app.id);
        const title = t(app.titleKey);
        return (
          <div key={app.id} className="tooltip" data-tip={title}>
            <button
              className={`btn btn-square btn-ghost relative ${focused === app.id ? "btn-active" : ""}`}
              aria-label={title}
              aria-pressed={focused === app.id}
              onClick={() => onActivate(app.id)}
            >
              <app.icon size={22} />
              {running && <span className="absolute bottom-0.5 size-1 rounded-full bg-primary" />}
            </button>
          </div>
        );
      })}
    </nav>
  );
}
