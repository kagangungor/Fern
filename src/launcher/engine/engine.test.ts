// @ts-expect-error -- bun:test is provided by the Bun test runner without @types/bun installed
import { describe, expect, test } from "bun:test";
import { fuzzyScore, normalizeTurkish } from "./fuzzy";
import { evaluateExpression } from "./calculator";
import { buildWebSearchUrl, detectDirectUrl } from "./web";
import { appKey } from "./apps";
import { buildQuickCommands, matchQuickCommands } from "./commands";

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

describe("apps", () => {
  test("builds stable unique appKey from path and name", () => {
    const keyA = appKey({
      name: "Visual Studio Code",
      path: "C:\\Users\\user\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe",
      icon: null,
      is_running: false,
    });
    const keyB = appKey({
      name: "Visual Studio Code",
      path: "C:\\Users\\user\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe",
      icon: "data:image/png;base64,abc",
      is_running: true,
      hwnd: 12345,
    });
    const keyC = appKey({
      name: "Code Insiders",
      path: "C:\\Users\\user\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe",
      icon: null,
      is_running: false,
    });
    expect(keyA).toBe(
      "C:\\Users\\user\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe\u0000Visual Studio Code"
    );
    expect(keyA).toBe(keyB);
    expect(keyA).not.toBe(keyC);
  });
});

describe("commands", () => {
  const enLabels: Record<string, string> = {
    "launcher.cmd.settings": "Fern Settings",
    "launcher.cmd.calendar": "Open Calendar",
    "launcher.cmd.micMute": "Toggle Microphone Mute",
    "launcher.cmd.notchToggle": "Toggle Notch",
    "launcher.cmd.restart": "Restart Fern",
    "launcher.cmd.taskmgr": "Task Manager",
    "launcher.cmd.desktop": "Show Desktop",
    "launcher.cmd.lock": "Lock Screen",
    "launcher.cmd.sleep": "Sleep",
    "launcher.cmd.emptyBin": "Empty Recycle Bin",
  };

  const trLabels: Record<string, string> = {
    "launcher.cmd.settings": "Fern Ayarları",
    "launcher.cmd.calendar": "Takvimi Aç",
    "launcher.cmd.micMute": "Mikrofonu Sustur / Aç",
    "launcher.cmd.notchToggle": "Çentiği Aç / Kapat",
    "launcher.cmd.restart": "Fern'ü Yeniden Başlat",
    "launcher.cmd.taskmgr": "Görev Yöneticisi",
    "launcher.cmd.desktop": "Masaüstünü Göster",
    "launcher.cmd.lock": "Ekranı Kilitle",
    "launcher.cmd.sleep": "Uyku Modu",
    "launcher.cmd.emptyBin": "Geri Dönüşüm Kutusunu Boşalt",
  };

  test("registers all 10 commands with confirmRequired only on destructive actions", () => {
    const commands = buildQuickCommands();
    expect(commands).toHaveLength(10);

    const byId = new Map(commands.map((c) => [c.id, c]));
    expect(byId.get("sys-empty-bin")?.confirmRequired).toBe(true);
    expect(byId.get("sys-sleep")?.confirmRequired).toBe(true);
    expect(byId.get("fern-settings")?.confirmRequired).toBeFalsy();
    expect(byId.get("sys-lock")?.confirmRequired).toBeFalsy();
    expect(byId.get("sys-taskmgr")?.confirmRequired).toBeFalsy();
  });

  test("matches bilingual EN/TR queries and Turkish diacritics", () => {
    const commands = buildQuickCommands();
    const tEn = (k: string) => enLabels[k] ?? k;
    const tTr = (k: string) => trLabels[k] ?? k;

    // English query against EN labels
    const lockEn = matchQuickCommands(commands, "lock", tEn);
    expect(lockEn[0]?.command.id).toBe("sys-lock");

    // Turkish query against EN labels (via bilingual keywords)
    const kilitleEn = matchQuickCommands(commands, "kilitle", tEn);
    expect(kilitleEn[0]?.command.id).toBe("sys-lock");

    // ASCII query matching Turkish diacritics ("çentik", "görev", "boşalt", "masaüstü")
    const centik = matchQuickCommands(commands, "centik", tTr);
    expect(centik[0]?.command.id).toBe("fern-notch-toggle");

    const gorev = matchQuickCommands(commands, "gorev", tTr);
    expect(gorev[0]?.command.id).toBe("sys-taskmgr");

    const bosalt = matchQuickCommands(commands, "bosalt", tTr);
    expect(bosalt[0]?.command.id).toBe("sys-empty-bin");

    const masaustu = matchQuickCommands(commands, "masaustu", tEn);
    expect(masaustu[0]?.command.id).toBe("sys-desktop");

    // Empty query yields no matches
    expect(matchQuickCommands(commands, "   ", tEn)).toEqual([]);
  });

  test("sorts matched commands by descending score", () => {
    const commands = buildQuickCommands();
    const tEn = (k: string) => enLabels[k] ?? k;
    const results = matchQuickCommands(commands, "fern", tEn);
    expect(results.length).toBeGreaterThanOrEqual(2);
    for (let i = 1; i < results.length; i++) {
      expect(results[i - 1].score).toBeGreaterThanOrEqual(results[i].score);
    }
  });
});
