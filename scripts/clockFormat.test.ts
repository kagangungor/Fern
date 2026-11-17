import { describe, expect, test } from "bun:test";
import { parseDuration, rtlDigitsToDisplay, rtlDigitsToSeconds } from "../src/clockFormat";

describe("clockFormat RTL odometer helpers", () => {
  test("rtlDigitsToDisplay formats right-to-left digits", () => {
    expect(rtlDigitsToDisplay("")).toBe("00:00");
    expect(rtlDigitsToDisplay("5")).toBe("00:05");
    expect(rtlDigitsToDisplay("130")).toBe("01:30");
    expect(rtlDigitsToDisplay("2500")).toBe("25:00");
    expect(rtlDigitsToDisplay("12500")).toBe("01:25:00");
  });

  test("rtlDigitsToSeconds converts right-to-left digits to seconds", () => {
    expect(rtlDigitsToSeconds("")).toBeNull();
    expect(rtlDigitsToSeconds("0")).toBeNull();
    expect(rtlDigitsToSeconds("45")).toBe(45);
    expect(rtlDigitsToSeconds("130")).toBe(90);
    expect(rtlDigitsToSeconds("2500")).toBe(1500);
    expect(rtlDigitsToSeconds("10000")).toBe(3600);
  });

  test("parseDuration still parses mm and mm:ss inputs", () => {
    expect(parseDuration("25")).toBe(1500);
    expect(parseDuration("1:30")).toBe(90);
  });
});
