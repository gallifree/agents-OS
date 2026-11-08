import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import fr from "./locales/fr.json";
import { readPref, writePref } from "./desktop/prefs";

export const languages = { en: "English", fr: "Français" } as const;
export type Language = keyof typeof languages;

const browser = navigator.language.slice(0, 2);
const initial = readPref("lang") ?? (browser in languages ? browser : "en");

i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, fr: { translation: fr } },
  lng: initial,
  fallbackLng: "en",
  interpolation: { escapeValue: false }, // React already escapes
});

document.documentElement.lang = i18n.language;
i18n.on("languageChanged", (lng) => {
  document.documentElement.lang = lng;
  writePref("lang", lng);
});

export default i18n;
