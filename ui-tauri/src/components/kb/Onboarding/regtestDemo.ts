import { canImportProjects, isImportProjectActive } from "@/daemon/transport";
import type { ImportedProjectIdentity } from "@/store/ui";

export interface RegtestStatusData {
  state_root?: string | null;
  data_root?: string | null;
  database?: string | null;
  database_encrypted?: boolean | null;
  current_workspace?: string | null;
  current_profile?: string | null;
  default_backend?: string | null;
  transactions?: number | null;
  wallets?: number | null;
}

/**
 * The regtest tile opens whatever book the daemon already has open. That book
 * is only an imported project when the user picked it with the folder picker;
 * otherwise it is the book the daemon was launched on (`KASSIBER_DEV_DATA_ROOT`
 * behind the Vite bridge). Marking a launch book as imported makes the app
 * layout try to activate a root the picker never approved, and makes every
 * later reset or "Back to setup" clear the daemon's data root.
 */
export function regtestDemoImportedProject(
  status: RegtestStatusData,
): ImportedProjectIdentity | undefined {
  const { state_root: stateRoot, data_root: dataRoot, database } = status;
  if (!canImportProjects() || !stateRoot || !dataRoot || !database) {
    return undefined;
  }
  if (!isImportProjectActive(dataRoot)) {
    return undefined;
  }
  return { stateRoot, dataRoot, database };
}
