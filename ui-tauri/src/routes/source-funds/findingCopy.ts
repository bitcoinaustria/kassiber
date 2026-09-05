import english from "@/i18n/locales/en/sourceFunds.json";
import type { SourceFundsFinding } from "./model";

/** Codes can have several distinct findings; unknown diagnostic variants stay verbatim. */
export function findingTranslationKeys(finding: SourceFundsFinding) {
  const known = (english.findings as Record<string, { title: string; nextStep: string; messages: Record<string, string> }>)[finding.code];
  const variant = known && Object.entries(known.messages).find(([, message]) => message === finding.message)?.[0];
  return {
    title: known ? `findings.${finding.code}.title` : undefined,
    message: variant ? `findings.${finding.code}.messages.${variant}` : undefined,
    nextStep: known?.nextStep === finding.next_step?.headline?.trim() ? `findings.${finding.code}.nextStep` : undefined,
  };
}
