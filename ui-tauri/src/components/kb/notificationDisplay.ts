function leadingProgressPhrase(text: string): string {
  return text
    .trim()
    .replace(/\s+/g, " ")
    .replace(/[.;]\s*$/, "")
    .split(/\s*(?:[;·])\s*/)[0]
    .trim()
    .toLocaleLowerCase();
}

export function shouldHideNotificationProgressLabel(
  body: string,
  progressLabel: string | null | undefined,
): boolean {
  if (!progressLabel?.trim()) return false;
  const bodyLead = leadingProgressPhrase(body);
  const labelLead = leadingProgressPhrase(progressLabel);
  return Boolean(bodyLead && labelLead && bodyLead === labelLead);
}
