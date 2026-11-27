// @ts-expect-error -- bun:test is provided by the Bun test runner without @types/bun installed
import { describe, expect, test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AiMarkdown, parseBlocks, parseInline } from "./AiMarkdown";

describe("AiMarkdown", () => {
  test("parses fenced code blocks with and without language tags", () => {
    const md = [
      "Before",
      "```ts",
      "const x = 1;",
      "console.log(x);",
      "```",
      "```",
      "plain",
      "```",
    ].join("\n");

    expect(parseBlocks(md)).toEqual([
      { type: "paragraph", text: "Before" },
      { type: "code", lang: "ts", code: "const x = 1;\nconsole.log(x);" },
      { type: "code", lang: "", code: "plain" },
    ]);
  });

  test("parses headings, bullet lists, numbered lists, blockquotes, and horizontal rules", () => {
    const md = [
      "# Title",
      "## Section",
      "### Sub",
      "",
      "- one",
      "* two",
      "",
      "1. first",
      "2. second",
      "",
      "> quoted line 1",
      "> quoted line 2",
      "",
      "---",
    ].join("\n");

    expect(parseBlocks(md)).toEqual([
      { type: "heading", level: 1, text: "Title" },
      { type: "heading", level: 2, text: "Section" },
      { type: "heading", level: 3, text: "Sub" },
      { type: "ul", items: ["one", "two"] },
      { type: "ol", items: ["first", "second"] },
      { type: "quote", lines: ["quoted line 1", "quoted line 2"] },
      { type: "hr" },
    ]);
  });

  test("parses inline bold, italic, code, strike, and links", () => {
    const tokens = parseInline(
      "Hi **bold** __also__ *italic* `code` ~~old~~ [Docs](https://example.com)"
    );
    expect(tokens).toEqual([
      { type: "text", text: "Hi " },
      { type: "bold", text: "bold" },
      { type: "text", text: " " },
      { type: "bold", text: "also" },
      { type: "text", text: " " },
      { type: "italic", text: "italic" },
      { type: "text", text: " " },
      { type: "code", text: "code" },
      { type: "text", text: " " },
      { type: "strike", text: "old" },
      { type: "text", text: " " },
      { type: "link", label: "Docs", url: "https://example.com" },
    ]);
  });

  test("renders links as non-navigating span.ai-md-link and never produces <a tags", () => {
    const html = renderToStaticMarkup(
      createElement(AiMarkdown, {
        text: "Visit [OpenAI](https://openai.com) and [Evil](javascript:alert(1)).",
      })
    );
    expect(html).not.toContain("<a");
    expect(html).toContain('class="ai-md-link"');
    expect(html).toContain("OpenAI");
    expect(html).toContain("Evil");
  });
});
