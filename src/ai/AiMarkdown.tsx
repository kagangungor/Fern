import { Fragment, useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";

export type Block =
  | { type: "code"; lang: string; code: string }
  | { type: "heading"; level: 1 | 2 | 3; text: string }
  | { type: "ul"; items: string[] }
  | { type: "ol"; items: string[] }
  | { type: "quote"; lines: string[] }
  | { type: "hr" }
  | { type: "paragraph"; text: string };

export type InlineToken =
  | { type: "text"; text: string }
  | { type: "bold"; text: string }
  | { type: "italic"; text: string }
  | { type: "code"; text: string }
  | { type: "strike"; text: string }
  | { type: "link"; label: string; url: string };

const HEADING_RE = /^(#{1,3})\s+(.*)$/;
const HR_RE = /^\s*(?:---+|\*\*\*+|___+)\s*$/;
const UL_RE = /^\s*[-*+]\s+(.*)$/;
const OL_RE = /^\s*\d+[.)]\s+(.*)$/;
const QUOTE_RE = /^\s*>\s?(.*)$/;
const FENCE_OPEN_RE = /^\s*```([^\s`]*)\s*$/;
const FENCE_CLOSE_RE = /^\s*```\s*$/;

function isBlockStart(line: string): boolean {
  return (
    FENCE_OPEN_RE.test(line) ||
    HEADING_RE.test(line) ||
    HR_RE.test(line) ||
    UL_RE.test(line) ||
    OL_RE.test(line) ||
    QUOTE_RE.test(line)
  );
}

export function parseBlocks(text: string): Block[] {
  const normalized = text.replace(/\r\n?/g, "\n");
  const lines = normalized.split("\n");
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i += 1;
      continue;
    }

    const fenceMatch = FENCE_OPEN_RE.exec(line);
    if (fenceMatch) {
      const lang = fenceMatch[1] ?? "";
      const codeLines: string[] = [];
      i += 1;
      while (i < lines.length && !FENCE_CLOSE_RE.test(lines[i])) {
        codeLines.push(lines[i]);
        i += 1;
      }
      if (i < lines.length && FENCE_CLOSE_RE.test(lines[i])) {
        i += 1;
      }
      blocks.push({ type: "code", lang, code: codeLines.join("\n") });
      continue;
    }

    const headingMatch = HEADING_RE.exec(line);
    if (headingMatch) {
      const level = Math.min(3, headingMatch[1].length) as 1 | 2 | 3;
      blocks.push({ type: "heading", level, text: headingMatch[2].trim() });
      i += 1;
      continue;
    }

    if (HR_RE.test(line)) {
      blocks.push({ type: "hr" });
      i += 1;
      continue;
    }

    if (QUOTE_RE.test(line)) {
      const quoteLines: string[] = [];
      while (i < lines.length) {
        const m = QUOTE_RE.exec(lines[i]);
        if (!m) break;
        quoteLines.push(m[1]);
        i += 1;
      }
      blocks.push({ type: "quote", lines: quoteLines });
      continue;
    }

    if (UL_RE.test(line)) {
      const items: string[] = [];
      while (i < lines.length) {
        const m = UL_RE.exec(lines[i]);
        if (!m) break;
        items.push(m[1]);
        i += 1;
      }
      blocks.push({ type: "ul", items });
      continue;
    }

    if (OL_RE.test(line)) {
      const items: string[] = [];
      while (i < lines.length) {
        const m = OL_RE.exec(lines[i]);
        if (!m) break;
        items.push(m[1]);
        i += 1;
      }
      blocks.push({ type: "ol", items });
      continue;
    }

    const paraLines: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !isBlockStart(lines[i])) {
      paraLines.push(lines[i]);
      i += 1;
    }
    blocks.push({ type: "paragraph", text: paraLines.join("\n") });
  }

  return blocks;
}

const INLINE_RE =
  /`([^`\n]+)`|\[([^\]\n]+)\]\(([^)\s]+)\)|\*\*([^*\n]+)\*\*|__([^_\n]+)__|~~([^~\n]+)~~|\*([^*\n]+)\*/g;

export function parseInline(text: string): InlineToken[] {
  const tokens: InlineToken[] = [];
  INLINE_RE.lastIndex = 0;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = INLINE_RE.exec(text)) !== null) {
    if (match.index > lastIndex) {
      tokens.push({ type: "text", text: text.slice(lastIndex, match.index) });
    }
    if (match[1] !== undefined) {
      tokens.push({ type: "code", text: match[1] });
    } else if (match[2] !== undefined && match[3] !== undefined) {
      tokens.push({ type: "link", label: match[2], url: match[3] });
    } else if (match[4] !== undefined) {
      tokens.push({ type: "bold", text: match[4] });
    } else if (match[5] !== undefined) {
      tokens.push({ type: "bold", text: match[5] });
    } else if (match[6] !== undefined) {
      tokens.push({ type: "strike", text: match[6] });
    } else if (match[7] !== undefined) {
      tokens.push({ type: "italic", text: match[7] });
    }
    lastIndex = INLINE_RE.lastIndex;
  }

  if (lastIndex < text.length) {
    tokens.push({ type: "text", text: text.slice(lastIndex) });
  }

  return tokens;
}

function renderInline(text: string) {
  const lines = text.split("\n");
  return lines.map((line, lineIdx) => (
    <Fragment key={lineIdx}>
      {lineIdx > 0 && <br />}
      {parseInline(line).map((tok, idx) => {
        switch (tok.type) {
          case "bold":
            return <strong key={idx}>{tok.text}</strong>;
          case "italic":
            return <em key={idx}>{tok.text}</em>;
          case "code":
            return (
              <code key={idx} className="ai-md-inline-code">
                {tok.text}
              </code>
            );
          case "strike":
            return <del key={idx}>{tok.text}</del>;
          case "link":
            return (
              <span key={idx} className="ai-md-link" title={tok.url}>
                {tok.label}
              </span>
            );
          default:
            return <Fragment key={idx}>{tok.text}</Fragment>;
        }
      })}
    </Fragment>
  ));
}

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    };
  }, []);

  const handleCopy = () => {
    navigator.clipboard
      ?.writeText(code)
      .then(() => {
        setCopied(true);
        if (timerRef.current !== null) clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };

  return (
    <div className="ai-md-code">
      <div className="ai-md-code-head">
        <span className="ai-md-code-lang">{lang || "text"}</span>
        <button
          type="button"
          className="ai-md-copy"
          onClick={handleCopy}
          aria-label="Copy code"
          title="Copy code"
        >
          {copied ? <Check size={12} /> : <Copy size={12} />}
        </button>
      </div>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}

export function AiMarkdown({ text }: { text: string }) {
  const blocks = parseBlocks(text);

  return (
    <div className="ai-md">
      {blocks.map((block, idx) => {
        switch (block.type) {
          case "code":
            return <CodeBlock key={idx} lang={block.lang} code={block.code} />;
          case "heading": {
            const content = renderInline(block.text);
            if (block.level === 1) return <h1 key={idx} className="ai-md-h1">{content}</h1>;
            if (block.level === 2) return <h2 key={idx} className="ai-md-h2">{content}</h2>;
            return <h3 key={idx} className="ai-md-h3">{content}</h3>;
          }
          case "ul":
            return (
              <ul key={idx} className="ai-md-ul">
                {block.items.map((item, itemIdx) => (
                  <li key={itemIdx}>{renderInline(item)}</li>
                ))}
              </ul>
            );
          case "ol":
            return (
              <ol key={idx} className="ai-md-ol">
                {block.items.map((item, itemIdx) => (
                  <li key={itemIdx}>{renderInline(item)}</li>
                ))}
              </ol>
            );
          case "quote":
            return (
              <blockquote key={idx} className="ai-md-quote">
                {renderInline(block.lines.join("\n"))}
              </blockquote>
            );
          case "hr":
            return <hr key={idx} className="ai-md-hr" />;
          case "paragraph":
            return (
              <p key={idx} className="ai-md-p">
                {renderInline(block.text)}
              </p>
            );
        }
      })}
    </div>
  );
}
