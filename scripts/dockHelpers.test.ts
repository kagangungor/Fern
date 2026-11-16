import { describe, expect, test } from "bun:test";
import {
  applyDockNameOverride,
  hideIdentifierForApp,
  isAppHiddenInDock,
  parseDockNames,
  resolveDockDisplayName,
} from "../src/dockHelpers";

describe("parseDockNames", () => {
  test("parses valid object map and rejects arrays or malformed JSON", () => {
    expect(parseDockNames('{"c:/app.exe":"Editor"}')).toEqual({ "c:/app.exe": "Editor" });
    expect(parseDockNames('["a","b"]')).toEqual({});
    expect(parseDockNames("{bad")).toEqual({});
    expect(parseDockNames(null)).toEqual({});
  });
});

describe("applyDockNameOverride", () => {
  test("stores trimmed custom name and deletes entry on empty or original match", () => {
    const added = applyDockNameOverride({}, "k1", "Visual Studio Code", "  VS Code  ");
    expect(added).toEqual({ k1: "VS Code" });

    const revertedSame = applyDockNameOverride(added, "k1", "Visual Studio Code", "Visual Studio Code");
    expect(revertedSame).toEqual({});

    const revertedEmpty = applyDockNameOverride(added, "k1", "Visual Studio Code", "   ");
    expect(revertedEmpty).toEqual({});

    const keepLocalizedDefaultClean = applyDockNameOverride(added, "k1", "Dosya Gezgini", "Dosya Gezgini");
    expect(keepLocalizedDefaultClean).toEqual({});

    const explicitEnglishOverride = applyDockNameOverride({}, "k1", "Dosya Gezgini", "File Explorer");
    expect(explicitEnglishOverride).toEqual({ k1: "File Explorer" });
  });
});

describe("resolveDockDisplayName", () => {
  test("prefers custom dock name override over localized default name", () => {
    const localize = (n: string) => `L(${n})`;
    expect(resolveDockDisplayName({ name: "Notepad" }, "notepad.exe", { "notepad.exe": "Notes" }, localize)).toBe("Notes");
    expect(resolveDockDisplayName({ name: "Notepad" }, "notepad.exe", {}, localize)).toBe("L(Notepad)");
  });
});

describe("isAppHiddenInDock & hideIdentifierForApp", () => {
  test("matches hidden identifiers by itemKey, path, executable, or name", () => {
    const app = { path: "C:\\Apps\\Foo.exe", executable: "Foo.exe", name: "Foo" };
    expect(isAppHiddenInDock(app, "c:/apps/foo.exe:foo.exe", [])).toBe(false);
    expect(isAppHiddenInDock(app, "c:/apps/foo.exe:foo.exe", ["foo.exe"])).toBe(true);
    expect(isAppHiddenInDock(app, "c:/apps/foo.exe:foo.exe", ["c:/apps/foo.exe:foo.exe"])).toBe(true);
    expect(isAppHiddenInDock(app, "c:/apps/foo.exe:foo.exe", ["Foo"])).toBe(true);
  });

  test("uses full itemKey for browser hosts so hiding one PWA does not hide the browser", () => {
    const pwa = { path: "C:\\Program Files\\Edge\\msedge.exe", executable: "msedge.exe", name: "YouTube" };
    const pwaKey = "c:/program files/edge/msedge.exe:youtube";
    const id = hideIdentifierForApp(pwa, pwaKey, true);
    expect(id).toBe(pwaKey);

    const regular = { path: "C:\\Tools\\git.exe", executable: "git.exe", name: "Git" };
    expect(hideIdentifierForApp(regular, "c:/tools/git.exe:git.exe", false)).toBe("git.exe");
  });
});
