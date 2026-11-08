import { useState } from "react";
import { useTranslation } from "react-i18next";
import { languages, type Language } from "../i18n";
import { applyTheme, savedTheme, type Theme } from "../desktop/prefs";

const themes: Theme[] = ["system", "light", "dark"];

export default function Settings() {
  const { t, i18n } = useTranslation();
  const [theme, setTheme] = useState(savedTheme);

  return (
    <div className="p-6">
      <h2 className="mb-4 text-lg font-semibold">{t("apps.settings.appearance")}</h2>

      <fieldset className="fieldset">
        <legend className="fieldset-legend">{t("apps.settings.theme")}</legend>
        <div className="join">
          {themes.map((th) => (
            <input
              key={th}
              type="radio"
              name="theme"
              className="join-item btn btn-sm"
              aria-label={t(`apps.settings.themes.${th}`)}
              checked={theme === th}
              onChange={() => {
                setTheme(th);
                applyTheme(th);
              }}
            />
          ))}
        </div>
      </fieldset>

      <fieldset className="fieldset mt-4">
        <legend className="fieldset-legend">{t("apps.settings.language")}</legend>
        <select
          className="select select-sm w-48"
          value={i18n.resolvedLanguage}
          onChange={(e) => i18n.changeLanguage(e.target.value as Language)}
        >
          {Object.entries(languages).map(([code, name]) => (
            <option key={code} value={code}>
              {name}
            </option>
          ))}
        </select>
      </fieldset>
    </div>
  );
}
