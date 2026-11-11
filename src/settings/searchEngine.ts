import { SETTINGS_SEARCH_INDEX, type SearchableSetting } from "./searchIndex";

export interface SearchResult {
  setting: SearchableSetting;
  title: string;
  desc?: string;
  score: number;
}

function normalize(text: string): string {
  return text
    .toLowerCase()
    .replace(/ğ/g, "g")
    .replace(/ü/g, "u")
    .replace(/ş/g, "s")
    .replace(/ı/g, "i")
    .replace(/ö/g, "o")
    .replace(/ç/g, "c")
    .replace(/[^a-z0-9\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function matchesWord(source: string, token: string): boolean {
  const words = source.split(" ");
  return words.some((w) => w.startsWith(token));
}

export function searchSettings(
  query: string,
  translate: (key: string) => string
): SearchResult[] {
  const q = normalize(query);
  if (!q) return [];

  const tokens = q.split(" ").filter((t) => t.length > 0);
  const results: SearchResult[] = [];

  for (const item of SETTINGS_SEARCH_INDEX) {
    const rawTitle = translate(item.titleKey);
    const rawDesc = item.descKey ? translate(item.descKey) : "";
    const tabName = translate(`tabs.${item.tab}`);

    const normTitle = normalize(rawTitle);
    const normDesc = normalize(rawDesc);
    const normTab = normalize(tabName);

    let totalScore = 0;
    let allTokensMatch = true;

    for (const token of tokens) {
      let tokenScore = 0;

      // Exact word prefix in title
      if (matchesWord(normTitle, token)) {
        tokenScore = normTitle.startsWith(token) ? 60 : 40;
      }
      // Exact word prefix in keywords
      else if (item.keywords.some((k) => matchesWord(normalize(k), token))) {
        tokenScore = 30;
      }
      // Exact word prefix in description
      else if (matchesWord(normDesc, token)) {
        tokenScore = 15;
      }
      // Exact word prefix in tab name
      else if (matchesWord(normTab, token)) {
        tokenScore = 10;
      }

      if (tokenScore === 0) {
        allTokensMatch = false;
        break;
      }
      totalScore += tokenScore;
    }

    if (allTokensMatch && totalScore > 0) {
      results.push({
        setting: item,
        title: rawTitle,
        desc: rawDesc,
        score: totalScore,
      });
    }
  }

  return results.sort((a, b) => b.score - a.score).slice(0, 8);
}
