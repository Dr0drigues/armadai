import { describe, it, expect } from "vitest";
import { fmtCost, fmtTokens } from "./api";

describe("formatters", () => {
  it("formats cost with 2 decimals and $", () => {
    expect(fmtCost(18.7)).toBe("$18.70");
  });
  it("groups token counts", () => {
    expect(fmtTokens(184210)).toBe("184 210");
  });
});
