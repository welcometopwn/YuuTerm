import { compileRegex } from "./highlight-policy.ts";
import type { Highlighting } from "./types";
export type Rule = Highlighting["rules"][number];
export type SyntaxPreview = {
  accepted: { line: number; source: string; rule: Rule }[];
  rejected: { line: number; reason: string }[];
};
export function reviewSyntax(value: SyntaxPreview): SyntaxPreview {
  const accepted: SyntaxPreview["accepted"] = [],
    rejected = [...value.rejected];
  for (const entry of value.accepted) {
    try {
      compileRegex(entry.rule.keywords);
      accepted.push(entry);
    } catch {
      rejected.push({
        line: entry.line,
        reason: "Unsupported by the bounded RE2 renderer",
      });
    }
  }
  return { accepted, rejected: rejected.sort((a, b) => a.line - b.line) };
}
