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
