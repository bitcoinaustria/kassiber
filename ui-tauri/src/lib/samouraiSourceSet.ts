export type SamouraiSection =
  | "deposit"
  | "badbank"
  | "premix"
  | "postmix";

export type SamouraiSourceFields = Record<SamouraiSection, string>;

export interface SamouraiSourceSet {
  network: string;
  children: Array<{
    section: SamouraiSection;
    script_type: string;
    root_path: string;
    descriptor: string;
    change_descriptor?: string;
  }>;
  xpubs: Array<{
    section: SamouraiSection;
    script_type: string;
    root_path: string;
    xpub: string;
  }>;
}

export interface SamouraiSourceSetBuildResult {
  sourceSet: SamouraiSourceSet;
  errors: Partial<Record<SamouraiSection, string>>;
}

const SECTION_ROOTS: Record<
  Exclude<SamouraiSection, "deposit">,
  number
> = {
  badbank: 2_147_483_644,
  premix: 2_147_483_645,
  postmix: 2_147_483_646,
};

const DESCRIPTOR_PREFIXES = [
  "pkh(",
  "sh(",
  "wpkh(",
  "wsh(",
  "tr(",
];

const EXTENDED_PUBLIC_KEY_PREFIXES = [
  "xpub",
  "tpub",
  "ypub",
  "upub",
  "zpub",
  "vpub",
];

function coinTypeFor(network?: string) {
  const normalized = String(network || "main").toLowerCase();
  return normalized === "main" || normalized === "mainnet" ? 0 : 1;
}

function rootPathFor(
  section: SamouraiSection,
  scriptType: string,
  network?: string,
) {
  const coinType = coinTypeFor(network);
  if (section === "deposit") {
    if (scriptType === "p2pkh") return `m/44'/${coinType}'/0'`;
    if (scriptType === "p2sh-p2wpkh") return `m/49'/${coinType}'/0'`;
    return `m/84'/${coinType}'/0'`;
  }
  return `m/84'/${coinType}'/${SECTION_ROOTS[section]}'`;
}

function scriptTypeFromPublicKey(material: string, section: SamouraiSection) {
  const prefix = material.slice(0, 4).toLowerCase();
  if (section !== "deposit") return "p2wpkh";
  if (prefix === "ypub" || prefix === "upub") return "p2sh-p2wpkh";
  if (prefix === "zpub" || prefix === "vpub") return "p2wpkh";
  return null;
}

function scriptTypeFromDescriptor(descriptor: string) {
  const normalized = descriptor.replace(/\s+/g, "").toLowerCase();
  if (normalized.startsWith("pkh(")) return "p2pkh";
  if (normalized.startsWith("sh(wpkh(")) return "p2sh-p2wpkh";
  return "p2wpkh";
}

function looksLikeDescriptor(value: string) {
  const lowered = value.toLowerCase();
  return DESCRIPTOR_PREFIXES.some((prefix) => lowered.startsWith(prefix));
}

function looksLikeExtendedPublicKey(value: string) {
  const lowered = value.toLowerCase();
  return EXTENDED_PUBLIC_KEY_PREFIXES.some((prefix) =>
    lowered.startsWith(prefix),
  );
}

function firstString(payload: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    const value = payload[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return undefined;
}

function descriptorsFromJson(material: string) {
  try {
    const payload = JSON.parse(material);
    if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
      return null;
    }
    const objectPayload = payload as Record<string, unknown>;
    let receive = firstString(objectPayload, [
      "descriptor",
      "receive_descriptor",
      "external_descriptor",
      "receive",
      "external",
    ]);
    let change = firstString(objectPayload, [
      "change_descriptor",
      "internal_descriptor",
      "change",
      "internal",
    ]);
    const descriptors = objectPayload.descriptors;
    if (Array.isArray(descriptors)) {
      for (const item of descriptors) {
        if (!item || typeof item !== "object" || Array.isArray(item)) continue;
        const descriptorItem = item as Record<string, unknown>;
        if (descriptorItem.active === false) continue;
        const descriptor = firstString(descriptorItem, ["desc", "descriptor"]);
        if (!descriptor) continue;
        if (descriptorItem.internal && !change) {
          change = descriptor;
        } else if (!descriptorItem.internal && !receive) {
          receive = descriptor;
        }
      }
    }
    return receive ? { descriptor: receive, changeDescriptor: change } : null;
  } catch {
    return null;
  }
}

function descriptorsFromText(material: string) {
  const lines = material
    .replace(/\r/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"));
  let receive: string | undefined;
  let change: string | undefined;
  const descriptors: string[] = [];
  for (const line of lines) {
    const delimiter = line.includes("=") ? "=" : line.includes(":") ? ":" : "";
    if (delimiter) {
      const [rawKey, ...rest] = line.split(delimiter);
      const key = rawKey.trim().toLowerCase().replace(/-/g, "_");
      const value = rest.join(delimiter).trim();
      if (
        [
          "descriptor",
          "receive",
          "receive_descriptor",
          "external",
          "external_descriptor",
        ].includes(key)
      ) {
        receive = value;
        continue;
      }
      if (
        [
          "change",
          "change_descriptor",
          "internal",
          "internal_descriptor",
        ].includes(key)
      ) {
        change = value;
        continue;
      }
    }
    if (looksLikeDescriptor(line)) descriptors.push(line);
  }
  if (!receive && descriptors.length > 0) receive = descriptors[0];
  if (!change && descriptors.length > 1) change = descriptors[1];
  return receive ? { descriptor: receive, changeDescriptor: change } : null;
}

function parseMaterial(material: string) {
  const trimmed = material.trim();
  if (!trimmed) return null;
  if (looksLikeExtendedPublicKey(trimmed)) return { xpub: trimmed };
  return descriptorsFromJson(trimmed) ?? descriptorsFromText(trimmed);
}

export function buildSamouraiSourceSet(
  fields: SamouraiSourceFields,
  network = "main",
): SamouraiSourceSetBuildResult {
  const sourceSet: SamouraiSourceSet = {
    network,
    children: [],
    xpubs: [],
  };
  const errors: Partial<Record<SamouraiSection, string>> = {};

  for (const section of [
    "deposit",
    "badbank",
    "premix",
    "postmix",
  ] satisfies SamouraiSection[]) {
    const material = fields[section].trim();
    if (!material) continue;
    const parsed = parseMaterial(material);
    if (!parsed) {
      errors[section] =
        "Paste an output descriptor, two receive/change descriptors, or an xpub/ypub/zpub.";
      continue;
    }
    if ("xpub" in parsed) {
      const scriptType = scriptTypeFromPublicKey(parsed.xpub, section);
      if (!scriptType) {
        errors[section] =
          "Bare Deposit xpub is ambiguous; paste a descriptor, ypub, or zpub instead.";
        continue;
      }
      sourceSet.xpubs.push({
        section,
        script_type: scriptType,
        root_path: rootPathFor(section, scriptType, network),
        xpub: parsed.xpub,
      });
      continue;
    }
    const scriptType = scriptTypeFromDescriptor(parsed.descriptor);
    sourceSet.children.push({
      section,
      script_type: scriptType,
      root_path: rootPathFor(section, scriptType, network),
      descriptor: parsed.descriptor,
      ...(parsed.changeDescriptor
        ? { change_descriptor: parsed.changeDescriptor }
        : {}),
    });
  }

  return { sourceSet, errors };
}
