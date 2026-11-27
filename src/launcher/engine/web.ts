import type { WebEngine } from "../types";

const HTTP_URL_RE = /^https?:\/\/\S+$/i;
const LOCALHOST_RE = /^localhost(:\d+)?(\/\S*)?$/i;
const DOMAIN_RE = /^([a-z0-9-]+(?:\.[a-z0-9-]+)+)(\/\S*)?$/i;
const VALID_TLD_RE = /^[a-z]{2,}$/i;

export function detectDirectUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || /\s/.test(trimmed)) return null;

  if (HTTP_URL_RE.test(trimmed)) {
    return trimmed;
  }

  if (LOCALHOST_RE.test(trimmed)) {
    return `http://${trimmed}`;
  }

  const domainMatch = DOMAIN_RE.exec(trimmed);
  if (domainMatch) {
    const host = domainMatch[1];
    const labels = host.split(".");
    const tld = labels[labels.length - 1];
    if (VALID_TLD_RE.test(tld)) {
      return `https://${trimmed}`;
    }
  }

  return null;
}

export function buildWebSearchUrl(query: string, engine: WebEngine): string {
  const encoded = encodeURIComponent(query.trim());
  if (engine === "duckduckgo") {
    return `https://duckduckgo.com/?q=${encoded}`;
  }
  return `https://www.google.com/search?q=${encoded}`;
}
