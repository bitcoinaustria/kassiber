import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

/**
 * The reviewer redaction corpora, frozen from the original Python
 * generators' RNG and distribution (plan 22): locked test data under
 * tests/fixtures/secret_redaction/, one gzip JSON-lines file per generator,
 * seed and count. Writes the cases to `file`.
 */
export function writeCorpus(name: string, seed: number, count: number, file: string): void {
  const path = fileURLToPath(
    new URL(`../../../tests/fixtures/secret_redaction/${name}-${seed}-${count}.jsonl.gz`, import.meta.url),
  );
  let bytes: Buffer;
  try {
    bytes = readFileSync(path);
  } catch {
    throw new Error(`${path} is missing: run scripts/fetch-test-data.sh`);
  }
  writeFileSync(file, gunzipSync(bytes));
}
