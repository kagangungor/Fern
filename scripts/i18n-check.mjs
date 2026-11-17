import fs from "fs";
import path from "path";

const LOCALES_DIR = path.resolve("src/locales");
const SOURCE_FILE = path.join(LOCALES_DIR, "en.json");

function extractPlaceholders(text) {
  const matches = text.match(/\{(\w+)\}/g) || [];
  return [...new Set(matches)].sort();
}

const source = JSON.parse(fs.readFileSync(SOURCE_FILE, "utf8"));
const sourceKeys = Object.keys(source);
const files = fs.readdirSync(LOCALES_DIR).filter((f) => f.endsWith(".json") && f !== "en.json");

let hasError = false;

for (const file of files) {
  const target = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, file), "utf8"));
  const targetKeys = Object.keys(target);

  const missing = sourceKeys.filter((k) => !(k in target) || String(target[k]).trim() === "");
  const extra = targetKeys.filter((k) => !(k in source));
  const placeholderMismatches = [];

  for (const key of sourceKeys) {
    if (!(key in target)) continue;
    const expected = extractPlaceholders(String(source[key]));
    const actual = extractPlaceholders(String(target[key]));
    if (expected.join(",") !== actual.join(",")) {
      placeholderMismatches.push({ key, expected, actual });
    }
  }

  console.log(`[i18n-check] ${file}: ${sourceKeys.length - missing.length}/${sourceKeys.length} keys`);
  if (missing.length > 0) {
    hasError = true;
    console.error(`  Missing/empty (${missing.length}):`, missing.slice(0, 15).join(", "));
  }
  if (extra.length > 0) {
    console.warn(`  Extra keys (${extra.length}):`, extra.slice(0, 15).join(", "));
  }
  if (placeholderMismatches.length > 0) {
    hasError = true;
    for (const m of placeholderMismatches) {
      console.error(`  Placeholder mismatch at "${m.key}": expected [${m.expected.join(", ")}], got [${m.actual.join(", ")}]`);
    }
  }
}

if (hasError) {
  process.exit(1);
}
console.log("[i18n-check] All locale files passed.");
