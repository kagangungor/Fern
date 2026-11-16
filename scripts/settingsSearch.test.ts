import { describe, expect, test } from "bun:test";
import en from "../src/locales/en.json";
import tr from "../src/locales/tr.json";
import { normalize, searchSettings } from "../src/settings/searchEngine";

const makeT = (dict: Record<string, string>) => (key: string) => dict[key] ?? en[key as keyof typeof en] ?? key;
const tEn = makeT(en);
const tTr = makeT(tr);

describe("settings searchEngine (#17 multilingual & fuzzy)", () => {
  test("normalize strips Turkish diacritics and punctuation cleanly", () => {
    expect(normalize("Şifre & Çentik — Işık!")).toBe("sifre centik isik");
  });

  test("finds settings despite minor typos via fuzzy matching", () => {
    const hits = searchSettings("tasaruf", tTr, "tr");
    expect(hits.some((r) => r.setting.id === "general-memory-saver")).toBe(true);
  });

  test("matches across English and Turkish catalogs regardless of active UI language", () => {
    const fromEnToTr = searchSettings("masaustu", tEn, "en");
    expect(fromEnToTr.some((r) => r.setting.id === "appearance-scale")).toBe(true);

    const fromTrToEn = searchSettings("desktop", tTr, "tr");
    expect(fromTrToEn.some((r) => r.setting.id === "appearance-scale")).toBe(true);
  });

  test("ranks exact title matches above cross-language descriptions", () => {
    const hits = searchSettings("scale", tEn, "en");
    expect(hits[0]?.setting.id).toBe("appearance-scale");
  });
});
