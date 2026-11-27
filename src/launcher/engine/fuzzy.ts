const TURKISH_MAP: Record<string, string> = {
  İ: "i",
  I: "i",
  ı: "i",
  Ç: "c",
  ç: "c",
  Ğ: "g",
  ğ: "g",
  Ö: "o",
  ö: "o",
  Ş: "s",
  ş: "s",
  Ü: "u",
  ü: "u",
};

export function normalizeTurkish(input: string): string {
  return input
    .replace(/[İIıÇçĞğÖöŞşÜü]/g, (ch) => TURKISH_MAP[ch] ?? ch)
    .toLowerCase()
    .replace(/\u0307/g, "");
}

export function fuzzyScore(target: string, query: string): number {
  const t = normalizeTurkish(target).trim();
  const q = normalizeTurkish(query).trim();
  if (!t || !q) return -1;

  const len = t.length;
  if (t === q) return 1200;
  if (t.startsWith(q)) return Math.max(1, 1000 - len);

  const words = t.split(/[\s\-_./\\]+/).filter(Boolean);
  for (let i = 0; i < words.length; i++) {
    if (words[i].startsWith(q)) {
      return Math.max(1, 850 - i * 12 - len);
    }
  }

  const at = t.indexOf(q);
  if (at !== -1) {
    return Math.max(1, 650 - at);
  }

  let qi = 0;
  for (let ti = 0; ti < len && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      qi++;
    }
  }
  if (qi === q.length) {
    return Math.max(1, 320 - len);
  }

  return -1;
}
