import { useResolvedColorMode } from "@/components/overview-dashboard/model";

/**
 * Chart colours for assistant visuals. Each set was run through the dataviz
 * palette validator against the card surface of its mode (white light,
 * #171717 dark): the categorical four pass lightness, chroma, CVD and
 * normal-vision separation; light-mode orange sits under 3:1, so every chart
 * keeps visible labels or its table view. Gains and losses (and received and
 * sent) use the blue/red pair, which separates under every CVD type; green
 * against red does not. Text never wears these colours.
 */
const PALETTES = {
  light: {
    series: ["#e8820c", "#0284c7", "#1f9d6a", "#6250d6"],
    positive: "#0284c7",
    negative: "#e0453f",
    muted: "#a3a3a3",
    fee: "#e0453f",
  },
  dark: {
    series: ["#d27a0b", "#1d9fd8", "#34ac7e", "#8f84e8"],
    positive: "#1d9fd8",
    negative: "#e66767",
    muted: "#5c5c5c",
    fee: "#e66767",
  },
} as const;

export type VisualPalette = (typeof PALETTES)["light" | "dark"];

export function useVisualPalette(): VisualPalette {
  return PALETTES[useResolvedColorMode()];
}

/**
 * A colour that follows the entity, never its rank: BTC is always the first
 * slot and Liquid the second, other assets take the remaining slots in the
 * order they first appear, and past four everything folds to the muted grey.
 */
export function seriesColorMap(names: string[], palette: VisualPalette): Map<string, string> {
  const fixed: Record<string, number> = { BTC: 0, LBTC: 1, "L-BTC": 1, LNBTC: 3 };
  const used = new Set<number>();
  const colors = new Map<string, string>();
  for (const name of names) {
    const slot = fixed[name.toUpperCase()];
    if (slot !== undefined && !used.has(slot)) {
      colors.set(name, palette.series[slot]);
      used.add(slot);
    }
  }
  for (const name of names) {
    if (colors.has(name)) continue;
    const slot = [0, 1, 2, 3].find((index) => !used.has(index));
    if (slot === undefined) {
      colors.set(name, palette.muted);
    } else {
      colors.set(name, palette.series[slot]);
      used.add(slot);
    }
  }
  return colors;
}
