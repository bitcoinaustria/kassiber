import { Color, MeshPhysicalMaterial } from "three";

// Runtime imports belong only in lazily loaded 3D scenes.
export type GlassTone = {
  color: string;
  attenuation: string;
  roughness: number;
  distance: number;
  /** Below 1 the base colour shows; a dark surface leaves full glass black. */
  transmission: number;
  glow: string;
  glowIntensity: number;
  rim: string;
};

export function glass(tone: GlassTone, thickness = 0.6) {
  return new MeshPhysicalMaterial({
    color: new Color(tone.color),
    metalness: 0,
    roughness: tone.roughness,
    transmission: tone.transmission,
    thickness,
    ior: 1.5,
    clearcoat: 1,
    clearcoatRoughness: 0.04,
    attenuationColor: new Color(tone.attenuation),
    attenuationDistance: tone.distance,
    specularIntensity: 1,
    emissive: new Color(tone.glow),
    emissiveIntensity: tone.glowIntensity,
    // A light rim on grazing edges reads as cut acrylic.
    sheen: 1,
    sheenColor: new Color(tone.rim),
    sheenRoughness: 0.25,
  });
}

export const LIGHT_TONES = {
  known: { color: "#d6e6ff", attenuation: "#2f7cf6", roughness: 0.04, distance: 1.1, transmission: 0.92, glow: "#000000", glowIntensity: 0, rim: "#ffffff" },
  estimated: { color: "#f5f7fa", attenuation: "#b8c2d0", roughness: 0.5, distance: 3, transmission: 0.85, glow: "#000000", glowIntensity: 0, rim: "#ffffff" },
  fee: { color: "#ffe9a8", attenuation: "#f59e0b", roughness: 0.06, distance: 0.8, transmission: 0.9, glow: "#000000", glowIntensity: 0, rim: "#fff7d6" },
  feeEstimated: { color: "#fbf3dc", attenuation: "#e7c77e", roughness: 0.5, distance: 2.5, transmission: 0.85, glow: "#000000", glowIntensity: 0, rim: "#fffaf0" },
  center: { color: "#f3f7fc", attenuation: "#8fb8f5", roughness: 0.02, distance: 3.5, transmission: 1, glow: "#000000", glowIntensity: 0, rim: "#ffffff" },
} satisfies Record<string, GlassTone>;

export const DARK_TONES = {
  known: { color: "#9cc4ff", attenuation: "#3b82f6", roughness: 0.05, distance: 1.4, transmission: 0.55, glow: "#1d4ed8", glowIntensity: 0.45, rim: "#cfe1ff" },
  estimated: { color: "#cbd3de", attenuation: "#94a3b8", roughness: 0.5, distance: 3, transmission: 0.45, glow: "#334155", glowIntensity: 0.4, rim: "#e2e8f0" },
  fee: { color: "#ffd466", attenuation: "#f59e0b", roughness: 0.06, distance: 1, transmission: 0.5, glow: "#b45309", glowIntensity: 0.6, rim: "#fff1c2" },
  feeEstimated: { color: "#e9dcb8", attenuation: "#c9a860", roughness: 0.5, distance: 2.5, transmission: 0.45, glow: "#57451f", glowIntensity: 0.4, rim: "#f6ecd2" },
  center: { color: "#dbe5f3", attenuation: "#60a5fa", roughness: 0.03, distance: 4, transmission: 0.62, glow: "#1e293b", glowIntensity: 0.5, rim: "#e0ecff" },
} satisfies Record<string, GlassTone>;

export function satin(color: string) {
  return new MeshPhysicalMaterial({
    color: new Color(color),
    metalness: 0.15,
    roughness: 0.32,
    clearcoat: 1,
    clearcoatRoughness: 0.12,
  });
}

