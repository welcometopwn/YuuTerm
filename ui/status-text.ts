// Splits long connection messages, such as the RDP control's
// "… for one of these reasons: 1) … 2) … 3) … Make sure … (code 516)",
// into parts that can be shown as a short summary and a readable list.
export interface StatusParts {
  lead: string;
  reasons: string[];
  advice: string;
  code: string;
}

// The control joins the last reason and its advice without a full stop.
const ADVICE = /\s+(?=(?:Make sure|Try|Contact|Check|Ask|If)\b)/;

export function statusParts(text: string): StatusParts {
  let rest = text.trim();
  let code = "";
  const codeMatch = /\s*\((code \d+)\)\.?$/.exec(rest);
  if (codeMatch) {
    code = codeMatch[1];
    rest = rest.slice(0, codeMatch.index);
  }
  const pieces = rest.split(/\s*\b\d\)\s+/);
  if (pieces.length < 3) return { lead: rest, reasons: [], advice: "", code };
  const lead = pieces[0].replace(/:$/, "").trim();
  const reasons = pieces.slice(1).map((r) => r.trim());
  let advice = "";
  const last = reasons[reasons.length - 1];
  const split = ADVICE.exec(last);
  if (split) {
    reasons[reasons.length - 1] = last.slice(0, split.index).trim();
    advice = last.slice(split.index).trim();
  }
  return {
    lead,
    reasons: reasons.map((r) => r.replace(/[.;,]$/, "")),
    advice,
    code,
  };
}

// One line for the pane bar; the full text stays in the pane.
export function statusSummary(text: string): string {
  const parts = statusParts(text);
  const lead = parts.reasons.length
    ? parts.lead.replace(/\s+for one of these reasons$/i, "")
    : parts.lead;
  return parts.code ? `${lead} (${parts.code})` : lead;
}
