import i18n from "./i18n";
import { create } from "zustand";

interface LocaleState {
  locale: "ja" | "en";
  setLocale: (l: "ja" | "en") => void;
}

export const useLocale = create<LocaleState>((set) => ({
  locale: (i18n.language as "ja" | "en") ?? "en",
  setLocale: (locale) => {
    void i18n.changeLanguage(locale);
    set({ locale });
  },
}));
