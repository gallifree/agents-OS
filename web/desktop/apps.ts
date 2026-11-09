import { lazy, type ComponentType, type LazyExoticComponent } from "react";
import { Activity, Bot, BrainCircuit, Info, MessagesSquare, Settings, type LucideIcon } from "lucide-react";
import type { Viewer } from "./App";
import type { Size } from "./windows";

export type AppProps = { viewer: Viewer };

export type AppDef = {
  id: string;
  /** i18n key of the window / dock title */
  titleKey: string;
  icon: LucideIcon;
  defaultSize: Size;
  /// Hidden from non-admins (the backend enforces access regardless).
  adminOnly?: boolean;
  // Apps load lazily: heavy ones (editors, canvases, terminals) must not weigh on the shell.
  component: LazyExoticComponent<ComponentType<AppProps>>;
};

export const apps: AppDef[] = [
  {
    id: "chat",
    titleKey: "apps.chat.title",
    icon: MessagesSquare,
    defaultSize: { w: 760, h: 560 },
    component: lazy(() => import("../apps/Chat")),
  },
  {
    id: "agents",
    titleKey: "apps.agents.title",
    icon: Bot,
    defaultSize: { w: 620, h: 560 },
    component: lazy(() => import("../apps/Agents")),
  },
  {
    id: "system",
    titleKey: "apps.system.title",
    icon: Activity,
    defaultSize: { w: 560, h: 520 },
    adminOnly: true,
    component: lazy(() => import("../apps/System")),
  },
  {
    id: "models",
    titleKey: "apps.models.title",
    icon: BrainCircuit,
    defaultSize: { w: 680, h: 560 },
    component: lazy(() => import("../apps/Models")),
  },
  {
    id: "settings",
    titleKey: "apps.settings.title",
    icon: Settings,
    defaultSize: { w: 600, h: 520 },
    component: lazy(() => import("../apps/Settings")),
  },
  {
    id: "about",
    titleKey: "apps.about.title",
    icon: Info,
    defaultSize: { w: 380, h: 280 },
    component: lazy(() => import("../apps/About")),
  },
];

export const appById = (id: string) => apps.find((a) => a.id === id);
