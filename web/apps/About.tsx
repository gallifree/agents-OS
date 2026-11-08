import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

type Health = "checking" | "online" | "offline";

export default function About() {
  const { t } = useTranslation();
  const [health, setHealth] = useState<Health>("checking");

  useEffect(() => {
    fetch("/health")
      .then((r) => setHealth(r.ok ? "online" : "offline"))
      .catch(() => setHealth("offline"));
  }, []);

  const badge = { checking: "badge-ghost", online: "badge-success", offline: "badge-error" }[health];

  return (
    <div className="flex flex-col items-center gap-3 p-8 text-center">
      <h2 className="text-2xl font-bold">rust-agents</h2>
      <p className="opacity-70">{t("apps.about.tagline")}</p>
      <p className="flex items-center gap-2">
        {t("apps.about.backend")}
        <span className={`badge badge-sm ${badge}`}>{t(`apps.about.${health}`)}</span>
      </p>
    </div>
  );
}
