/**
 * Collision-free keys for picker rows.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/modelPickerKeys.ts`. The provider name is
 * length-prefixed, so a provider called `a:b` with model `c` cannot collide with
 * provider `a` and model `b:c` — model ids routinely contain `:` and `/`.
 */

const MODEL_KEY_PREFIX = "model:";

export function modelPickerModelKey(provider: string, model: string): string {
  return `${MODEL_KEY_PREFIX}${provider.length}:${provider}${model}`;
}

export function parseModelPickerModelKey(
  key: string,
): { provider: string; model: string } | null {
  if (!key.startsWith(MODEL_KEY_PREFIX)) return null;
  const encoded = key.slice(MODEL_KEY_PREFIX.length);
  const separatorIndex = encoded.indexOf(":");
  if (separatorIndex === -1) return null;

  const lengthText = encoded.slice(0, separatorIndex);
  if (!/^\d+$/.test(lengthText)) return null;

  const providerLength = Number(lengthText);
  const value = encoded.slice(separatorIndex + 1);
  if (!Number.isSafeInteger(providerLength) || providerLength > value.length) {
    return null;
  }
  return {
    provider: value.slice(0, providerLength),
    model: value.slice(providerLength),
  };
}

/** The picker's left-rail selection: the Favorites list or one provider. */
export const FAVORITES_VIEW = "favorites" as const;
export type ModelPickerView = typeof FAVORITES_VIEW | { provider: string };

export function modelPickerViewKey(view: ModelPickerView): string {
  return view === FAVORITES_VIEW ? FAVORITES_VIEW : `provider:${view.provider}`;
}

/** Where the picker opens: Favorites if the current model is starred, else its provider. */
export function initialModelPickerView(input: {
  groups: ReadonlyArray<{ provider: { name: string; is_default?: boolean } }>;
  value: { provider: string; model: string } | null;
  favorites: ReadonlyArray<{ provider: string; model: string }>;
}): ModelPickerView {
  const { groups, value, favorites } = input;
  if (
    value &&
    favorites.some(
      (favorite) =>
        favorite.provider === value.provider && favorite.model === value.model,
    )
  ) {
    return FAVORITES_VIEW;
  }
  const current = value
    ? groups.find(({ provider }) => provider.name === value.provider)
    : undefined;
  const fallback =
    current ?? groups.find(({ provider }) => provider.is_default) ?? groups[0];
  return fallback ? { provider: fallback.provider.name } : FAVORITES_VIEW;
}

/** Toggles the model picker while the composer has focus (T3 Code's binding). */
export const MODEL_PICKER_SHORTCUT = ["mod", "shift", "m"] as const;
