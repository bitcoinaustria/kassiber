import { SettingsScreen } from "@/components/kb/SettingsScreen";
import type { SettingsSectionId } from "@/components/kb/settingsSections";

/**
 * One settings category. Every `/settings/<slug>` route renders this with its
 * own `section`; the category navigation itself lives in the side nav.
 */
export function Settings({ section }: { section: SettingsSectionId }) {
  return <SettingsScreen sectionId={section} />;
}
