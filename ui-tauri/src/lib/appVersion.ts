export const APP_VERSION = __APP_VERSION__;
export const APP_COMMIT = __APP_COMMIT__;

export function appVersionLabel(): string {
  return `${APP_VERSION} (${APP_COMMIT ? APP_COMMIT.slice(0, 7) : "unknown"})`;
}
