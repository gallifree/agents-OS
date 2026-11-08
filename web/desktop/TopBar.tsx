import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { graphql, useMutation } from "react-relay";
import { CircleUser, LogOut } from "lucide-react";
import type { Viewer } from "./App";
import type { TopBarLogoutMutation } from "./__generated__/TopBarLogoutMutation.graphql";

const logoutMutation = graphql`
  mutation TopBarLogoutMutation {
    logout
  }
`;

export function TopBar({ title, viewer }: { title?: string; viewer: Viewer }) {
  const { t, i18n } = useTranslation();
  const [now, setNow] = useState(() => new Date());
  const [logout, loggingOut] = useMutation<TopBarLogoutMutation>(logoutMutation);

  useEffect(() => {
    const timer = setInterval(() => setNow(new Date()), 10_000);
    return () => clearInterval(timer);
  }, []);

  const clock = new Intl.DateTimeFormat(i18n.language, {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(now);

  // A full reload drops the Relay store, open windows and the WebSocket: nothing of this user stays in memory.
  const signOut = () => logout({ variables: {}, onCompleted: () => location.reload(), onError: () => location.reload() });

  return (
    <header className="flex h-8 shrink-0 items-center justify-between border-b border-base-300 bg-base-100/80 px-3 text-sm backdrop-blur select-none">
      <span className="font-semibold">rust-agents</span>
      <span className="truncate px-4 opacity-70">{title}</span>
      <div className="flex items-center gap-3">
        <time dateTime={now.toISOString()}>{clock}</time>
        <div className="dropdown dropdown-end">
          <button className="btn btn-ghost btn-xs gap-1" aria-label={t("desktop.account")}>
            <CircleUser size={14} />
            <span className="max-sm:hidden">{viewer.username}</span>
          </button>
          <ul className="dropdown-content menu z-[100000] mt-1 w-44 rounded-box border border-base-300 bg-base-100 p-1 shadow-lg">
            <li>
              <button onClick={signOut} disabled={loggingOut}>
                <LogOut size={14} /> {t("desktop.signOut")}
              </button>
            </li>
          </ul>
        </div>
      </div>
    </header>
  );
}
