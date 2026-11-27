type Operator = "+" | "-" | "*" | "/" | "%" | "^" | "u-" | "u+";

type Token =
  | { type: "num"; value: number }
  | { type: "op"; op: Operator }
  | { type: "lparen" }
  | { type: "rparen" };

const PRECEDENCE: Record<Operator, number> = {
  "+": 1,
  "-": 1,
  "*": 2,
  "/": 2,
  "%": 2,
  "u-": 3,
  "u+": 3,
  "^": 4,
};

function isRightAssociative(op: Operator): boolean {
  return op === "^" || op === "u-" || op === "u+";
}

function isUnary(op: Operator): boolean {
  return op === "u-" || op === "u+";
}

function tokenize(expr: string): { tokens: Token[]; hasBinaryOp: boolean } | null {
  const tokens: Token[] = [];
  let hasBinaryOp = false;
  let i = 0;

  while (i < expr.length) {
    const ch = expr[i];

    if (ch === " " || ch === "\t") {
      i++;
      continue;
    }

    if ((ch >= "0" && ch <= "9") || ch === ".") {
      const prev = tokens[tokens.length - 1];
      if (prev && (prev.type === "num" || prev.type === "rparen")) {
        return null;
      }
      let start = i;
      let dotCount = 0;
      while (i < expr.length && ((expr[i] >= "0" && expr[i] <= "9") || expr[i] === ".")) {
        if (expr[i] === ".") dotCount++;
        i++;
      }
      const numStr = expr.slice(start, i);
      if (dotCount > 1 || numStr === ".") return null;
      const value = Number(numStr);
      if (!Number.isFinite(value)) return null;
      tokens.push({ type: "num", value });
      continue;
    }

    if (ch === "(") {
      const prev = tokens[tokens.length - 1];
      if (prev && (prev.type === "num" || prev.type === "rparen")) {
        return null;
      }
      tokens.push({ type: "lparen" });
      i++;
      continue;
    }

    if (ch === ")") {
      const prev = tokens[tokens.length - 1];
      if (!prev || prev.type === "op" || prev.type === "lparen") {
        return null;
      }
      tokens.push({ type: "rparen" });
      i++;
      continue;
    }

    if (ch === "+" || ch === "-") {
      const prev = tokens[tokens.length - 1];
      if (!prev || prev.type === "op" || prev.type === "lparen") {
        tokens.push({ type: "op", op: ch === "-" ? "u-" : "u+" });
      } else {
        hasBinaryOp = true;
        tokens.push({ type: "op", op: ch });
      }
      i++;
      continue;
    }

    if (ch === "*" || ch === "/" || ch === "%" || ch === "^") {
      const prev = tokens[tokens.length - 1];
      if (!prev || prev.type === "op" || prev.type === "lparen") {
        return null;
      }
      hasBinaryOp = true;
      tokens.push({ type: "op", op: ch });
      i++;
      continue;
    }

    return null;
  }

  return { tokens, hasBinaryOp };
}

function toRpn(tokens: Token[]): Token[] | null {
  const output: Token[] = [];
  const opStack: (Token & { type: "op" | "lparen" })[] = [];

  for (const token of tokens) {
    if (token.type === "num") {
      output.push(token);
      continue;
    }

    if (token.type === "lparen") {
      opStack.push(token);
      continue;
    }

    if (token.type === "rparen") {
      let foundLeft = false;
      while (opStack.length > 0) {
        const top = opStack.pop()!;
        if (top.type === "lparen") {
          foundLeft = true;
          break;
        }
        output.push(top);
      }
      if (!foundLeft) return null;
      continue;
    }

    if (isUnary(token.op)) {
      opStack.push(token);
      continue;
    }

    while (opStack.length > 0) {
      const top = opStack[opStack.length - 1];
      if (top.type !== "op") break;
      const topPrec = PRECEDENCE[top.op];
      const curPrec = PRECEDENCE[token.op];
      const shouldPop = isRightAssociative(token.op) ? topPrec > curPrec : topPrec >= curPrec;
      if (!shouldPop) break;
      output.push(opStack.pop()!);
    }
    opStack.push(token);
  }

  while (opStack.length > 0) {
    const top = opStack.pop()!;
    if (top.type === "lparen") return null;
    output.push(top);
  }

  return output;
}

function evalRpn(rpn: Token[]): number | null {
  const stack: number[] = [];

  for (const token of rpn) {
    if (token.type === "num") {
      stack.push(token.value);
      continue;
    }

    if (token.type !== "op") return null;

    if (isUnary(token.op)) {
      if (stack.length < 1) return null;
      const a = stack.pop()!;
      stack.push(token.op === "u-" ? -a : a);
      continue;
    }

    if (stack.length < 2) return null;
    const b = stack.pop()!;
    const a = stack.pop()!;
    let res: number;

    switch (token.op) {
      case "+":
        res = a + b;
        break;
      case "-":
        res = a - b;
        break;
      case "*":
        res = a * b;
        break;
      case "/":
        if (b === 0) return null;
        res = a / b;
        break;
      case "%":
        if (b === 0) return null;
        res = a % b;
        break;
      case "^":
        res = Math.pow(a, b);
        break;
      default:
        return null;
    }

    if (!Number.isFinite(res)) return null;
    stack.push(res);
  }

  if (stack.length !== 1) return null;
  return stack[0];
}

export function evaluateExpression(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  const normalized = trimmed
    .replace(/×/g, "*")
    .replace(/÷/g, "/")
    .replace(/,/g, ".")
    .replace(/(?<=[\d)])\s*[xX]\s*(?=[(\d-])/g, "*");

  if (!/^[0-9.+\-*/%^()\s]+$/.test(normalized)) {
    return null;
  }

  const lexed = tokenize(normalized);
  if (!lexed || !lexed.hasBinaryOp || lexed.tokens.length === 0) {
    return null;
  }

  const rpn = toRpn(lexed.tokens);
  if (!rpn) return null;

  const result = evalRpn(rpn);
  if (result === null || !Number.isFinite(result)) return null;

  const rounded = Math.round(result * 1e10) / 1e10;
  if (!Number.isFinite(rounded)) return null;

  const clean = Object.is(rounded, -0) ? 0 : rounded;
  return String(clean);
}
