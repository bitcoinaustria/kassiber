const GOOGLE_WORKSPACE_ROUTES: Array<[RegExp, string]> = [
  [/^\/document\/d\//, "Google Doc"],
  [/^\/spreadsheets\/d\//, "Google Sheet"],
  [/^\/presentation\/d\//, "Google Slides deck"],
  [/^\/forms\/d\//, "Google Form"],
  [/^\/drawings\/d\//, "Google Drawing"],
  [/^\/file\/d\//, "Google Drive file"],
  [/^\/drive\/folders\//, "Google Drive folder"],
];

function clean(value: string | null | undefined): string {
  return (value ?? "").replace(/\s+/g, " ").trim();
}

function parseUrl(raw: string | null | undefined): URL | null {
  const value = clean(raw);
  if (!value) return null;
  try {
    return new URL(value);
  } catch {
    return null;
  }
}

export function displayAttachmentUrl(raw: string | null | undefined): string {
  const parsed = parseUrl(raw);
  if (!parsed) return clean(raw);
  const host = parsed.hostname.replace(/^www\./i, "");
  const path = parsed.pathname === "/" ? "" : parsed.pathname.replace(/\/$/, "");
  return `${host}${path}`;
}

function pathTitle(pathname: string): string | null {
  const parts = pathname
    .split("/")
    .map((part) => part.trim())
    .filter(Boolean);
  const candidate = parts.at(-1);
  if (!candidate || /^[a-f0-9-]{16,}$/i.test(candidate)) return null;
  try {
    return decodeURIComponent(candidate)
      .replace(/\.[a-z0-9]{2,5}$/i, "")
      .replace(/[-_+]+/g, " ")
      .replace(/\s+/g, " ")
      .trim();
  } catch {
    return candidate.replace(/[-_+]+/g, " ").trim();
  }
}

export function fallbackUrlAttachmentLabel(raw: string): string {
  const parsed = parseUrl(raw);
  if (!parsed) return clean(raw) || "Link attachment";
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return "Link attachment";
  }

  const host = parsed.hostname.replace(/^www\./i, "");
  if (host === "docs.google.com" || host === "drive.google.com") {
    const route = GOOGLE_WORKSPACE_ROUTES.find(([pattern]) =>
      pattern.test(parsed.pathname),
    );
    return (
      route?.[1] ??
      (host === "drive.google.com"
        ? "Google Drive link"
        : "Google Workspace link")
    );
  }

  const title = pathTitle(parsed.pathname);
  if (title && title.toLowerCase() !== host.toLowerCase()) {
    return `${host} - ${title}`;
  }
  return host || "Link attachment";
}

export function urlAttachmentLabel(
  rawUrl: string | null | undefined,
  savedLabel: string | null | undefined,
): string {
  const label = clean(savedLabel);
  const url = clean(rawUrl);
  if (!url) return label || "Link attachment";
  const displayUrl = displayAttachmentUrl(url);
  const parsed = parseUrl(url);
  const canonicalUrl = parsed?.toString() ?? url;
  if (
    label &&
    label !== url &&
    label !== canonicalUrl &&
    label !== displayUrl
  ) {
    return label;
  }
  return fallbackUrlAttachmentLabel(url);
}
