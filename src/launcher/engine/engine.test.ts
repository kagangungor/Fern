// @ts-expect-error -- bun:test is provided by the Bun test runner without @types/bun installed
import { describe, expect, test } from "bun:test";
import { fuzzyScore, normalizeTurkish } from "./fuzzy";
import { evaluateExpression } from "./calculator";
import { buildWebSearchUrl, detectDirectUrl } from "./web";

describe("fuzzy", () => {
  test("folds Turkish diacritics", () => {
    expect(normalizeTurkish("Çeviri Şifre Işık İSTANBUL")).toBe("ceviri sifre isik istanbul");
    expect(normalizeTurkish("Ğğ Öö Üü Çç Şş İı I")).toBe("gg oo uu cc ss ii i");
  });

  test("ranks exact > prefix > word-start > substring > subsequence", () => {
    const exact = fuzzyScore("Code", "code");
    const prefix = fuzzyScore("Code Runner", "code");
    const wordStart = fuzzyScore("Visual Studio Code", "code");
    const substring = fuzzyScore("Decode Tool", "code");
    const subseq = fuzzyScore("Visual Studio Code", "vsc");
    expect(exact).toBeGreaterThan(prefix);
    expect(prefix).toBeGreaterThan(wordStart);
    expect(wordStart).toBeGreaterThan(substring);
    expect(substring).toBeGreaterThan(subseq);
    expect(subseq).toBeGreaterThan(0);
    expect(fuzzyScore("Spotify", "zzz")).toBe(-1);
  });

  test("matches Turkish words with ASCII queries", () => {
    expect(fuzzyScore("Görev Yöneticisi", "gorev")).toBeGreaterThan(0);
    expect(fuzzyScore("Geri Dönüşüm Kutusu", "donusum")).toBeGreaterThan(0);
    expect(fuzzyScore("İstanbul Saat", "istanbul")).toBeGreaterThan(0);
    expect(fuzzyScore("Işık Ayarları", "isik")).toBeGreaterThan(0);
  });

  test("handles empty inputs gracefully", () => {
    expect(fuzzyScore("Code", "")).toBe(-1);
    expect(fuzzyScore("", "code")).toBe(-1);
    expect(fuzzyScore("Code", "   ")).toBe(-1);
  });
});

describe("calculator", () => {
  test("evaluates precedence, parentheses, unary minus, and comma decimals", () => {
    expect(evaluateExpression("2 + 3 * 4")).toBe("14");
    expect(evaluateExpression("(2 + 3) * 4")).toBe("20");
    expect(evaluateExpression("2 * -3")).toBe("-6");
    expect(evaluateExpression("-(2 + 3)")).toBe("-5");
    expect(evaluateExpression("12,5 * 4")).toBe("50");
    expect(evaluateExpression("2^10")).toBe("1024");
    expect(evaluateExpression("2^3^2")).toBe("512");
    expect(evaluateExpression("10 % 3")).toBe("1");
    expect(evaluateExpression("0.1 + 0.2")).toBe("0.3");
  });

  test("normalizes ×, ÷, and x between numbers", () => {
    expect(evaluateExpression("6 × 7")).toBe("42");
    expect(evaluateExpression("20 ÷ 4")).toBe("5");
    expect(evaluateExpression("6 x 7")).toBe("42");
    expect(evaluateExpression("6x7")).toBe("42");
  });

  test("rejects plain numbers, division by zero, overflow, and non-math text", () => {
    expect(evaluateExpression("42")).toBeNull();
    expect(evaluateExpression("-42")).toBeNull();
    expect(evaluateExpression("(42)")).toBeNull();
    expect(evaluateExpression("10 / 0")).toBeNull();
    expect(evaluateExpression("10 % 0")).toBeNull();
    expect(evaluateExpression("10^1000")).toBeNull();
    expect(evaluateExpression("process.exit(1)")).toBeNull();
    expect(evaluateExpression("(2 + 3")).toBeNull();
    expect(evaluateExpression("2 + 3)")).toBeNull();
    expect(evaluateExpression("2 +")).toBeNull();
    expect(evaluateExpression("")).toBeNull();
  });
});

describe("web", () => {
  test("detects URLs and localhost without false positives", () => {
    expect(detectDirectUrl("https://github.com/kagangungor/Fern")).toBe(
      "https://github.com/kagangungor/Fern"
    );
    expect(detectDirectUrl("http://example.com/test?a=1")).toBe("http://example.com/test?a=1");
    expect(detectDirectUrl("github.com")).toBe("https://github.com");
    expect(detectDirectUrl("docs.rs/tauri/latest")).toBe("https://docs.rs/tauri/latest");
    expect(detectDirectUrl("localhost:1420")).toBe("http://localhost:1420");
    expect(detectDirectUrl("localhost:3000/api/health")).toBe("http://localhost:3000/api/health");
    expect(detectDirectUrl("localhost")).toBe("http://localhost");
    expect(detectDirectUrl("visual studio code")).toBeNull();
    expect(detectDirectUrl("2.5+3")).toBeNull();
    expect(detectDirectUrl("version.1")).toBeNull();
    expect(detectDirectUrl("a.b")).toBeNull();
    expect(detectDirectUrl("")).toBeNull();
  });

  test("builds Google and DuckDuckGo search URLs", () => {
    expect(buildWebSearchUrl("fern tauri", "google")).toBe(
      "https://www.google.com/search?q=fern%20tauri"
    );
    expect(buildWebSearchUrl("fern tauri", "duckduckgo")).toBe(
      "https://duckduckgo.com/?q=fern%20tauri"
    );
  });
});
