import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function TopBar({ title }: { title?: string }) {
  const { i18n } = useTranslation();
  const [now, setNow] = useState(() => new Date());

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

  return (
    <header className="flex h-8 shrink-0 items-center justify-between border-b border-base-300 bg-base-100/80 px-3 text-sm backdrop-blur select-none">
      <span className="font-semibold">rust-agents</span>
      <span className="truncate px-4 opacity-70">{title}</span>
      <time dateTime={now.toISOString()}>{clock}</time>
    </header>
  );
}
