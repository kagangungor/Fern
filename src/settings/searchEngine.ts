import { LANGUAGES, dateLocale } from "../i18n";
import { SETTINGS_SEARCH_INDEX, type SearchableSetting } from "./searchIndex";

export interface SearchResult {
  setting: SearchableSetting;
  title: string;
  desc?: string;
  score: number;
}

export function normalize(text: string): string {
  return text
    .toLowerCase()
    .replace(/ı/g, "i")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function distance(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    let best = row[0];
    for (let j = 1; j <= b.length; j++) {
      row[j] = Math.min(prev[j] + 1, row[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (row[j] < best) best = row[j];
    }
    if (best > max) return max + 1;
    prev = row;
  }
  return prev[b.length];
}

function tokenScore(token: string, text: string): number {
  if (!text) return 0;
  const words = text.split(" ");
  let best = 0;
  for (const word of words) {
    if (word === token) best = Math.max(best, 1);
    else if (word.startsWith(token)) best = Math.max(best, 0.85);
    else if (token.length >= 3 && word.includes(token)) best = Math.max(best, 0.55);
    else if (token.length >= 4 && word.length >= 4) {
      const allowed = token.length >= 8 ? 2 : 1;
      const d = distance(token, word.slice(0, token.length + 1), allowed);
      if (d <= allowed) best = Math.max(best, 0.5 - d * 0.1);
    }
  }
  return best;
}

interface PreparedSetting {
  item: SearchableSetting;
  fields: { text: string; weight: number }[];
}

const preparedCache = new Map<string, PreparedSetting[]>();

function getPrepared(activeLang: string): PreparedSetting[] {
  const cached = preparedCache.get(activeLang);
  if (cached) return cached;

  const entries = SETTINGS_SEARCH_INDEX.map((item) => {
    const fields: { text: string; weight: number }[] = [];
    for (const [code, lang] of Object.entries(LANGUAGES)) {
      const langWeight = code === activeLang ? 1 : code === "en" ? 0.85 : 0.75;
      const label = lang.messages[item.titleKey];
      const desc = item.descKey ? lang.messages[item.descKey] : undefined;
      const tabLabel = lang.messages[`tabs.${item.tab}`];
      if (label) fields.push({ text: normalize(label), weight: 3 * langWeight });
      if (desc) fields.push({ text: normalize(desc), weight: 1 * langWeight });
      if (tabLabel) fields.push({ text: normalize(tabLabel), weight: 0.5 * langWeight });
    }
    if (item.keywords.length > 0) {
      fields.push({ text: normalize(item.keywords.join(" ")), weight: 1.5 });
    }
    return { item, fields };
  });

  preparedCache.set(activeLang, entries);
  return entries;
}

export function searchSettings(
  query: string,
  translate: (key: string) => string,
  activeLang = dateLocale()
): SearchResult[] {
  const q = normalize(query);
  if (!q) return [];

  const tokens = q.split(" ").filter((t) => t.length > 0);
  const results: SearchResult[] = [];

  for (const { item, fields } of getPrepared(activeLang)) {
    let totalScore = 0;
    let allTokensMatch = true;

    for (const token of tokens) {
      let best = 0;
      for (const field of fields) {
        const score = tokenScore(token, field.text) * field.weight;
        if (score > best) best = score;
      }
      if (best === 0) {
        allTokensMatch = false;
        break;
      }
      totalScore += best;
    }

    if (allTokensMatch && totalScore > 0) {
      results.push({
        setting: item,
        title: translate(item.titleKey),
        desc: item.descKey ? translate(item.descKey) : undefined,
        score: totalScore / tokens.length,
      });
    }
  }

  return results.sort((a, b) => b.score - a.score).slice(0, 8);
}
