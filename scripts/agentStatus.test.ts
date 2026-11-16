import { describe, expect, test } from "bun:test";
import { advanceAskPick, shouldAgentAutoOpen } from "../src/components/AgentStatus";

describe("agentStatus helpers", () => {
  test("shouldAgentAutoOpen expands only for needs_input and error", () => {
    expect(shouldAgentAutoOpen(null)).toBe(false);
    expect(shouldAgentAutoOpen({ state: "working" })).toBe(false);
    expect(shouldAgentAutoOpen({ state: "tool" })).toBe(false);
    expect(shouldAgentAutoOpen({ state: "done" })).toBe(false);
    expect(shouldAgentAutoOpen({ state: "needs_input" })).toBe(true);
    expect(shouldAgentAutoOpen({ state: "error" })).toBe(true);
  });

  test("advanceAskPick steps through questions and finishes on last choice", () => {
    const step1 = advanceAskPick([], 1, 2);
    expect(step1).toEqual({ done: false, next: [1] });

    const step2 = advanceAskPick(step1.next, 0, 2);
    expect(step2).toEqual({ done: true, next: [1, 0] });

    const single = advanceAskPick([], 2, 1);
    expect(single).toEqual({ done: true, next: [2] });
  });
});
