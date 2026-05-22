import { AlertTriangle, Archive, FileInput, HardDrive, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { CommandLine, PathField } from "./SettingsControls";
import type { StatusData } from "./SettingsModel";

export function DataSettingsPanel({
  status,
  onOpenImports,
}: {
  status: StatusData | null;
  onOpenImports: () => void;
}) {
  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <FileInput className="size-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-semibold">Import data</h3>
        </div>
        <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <p className="max-w-prose text-sm text-muted-foreground">
            Bring in exchange CSV exports, on-chain history, and BIP-329 labels
            from the import tools.
          </p>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={onOpenImports}
          >
            <FileInput className="size-4" aria-hidden="true" />
            Open import tools
          </Button>
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Archive className="size-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-semibold">Backup &amp; restore</h3>
        </div>
        <div className="space-y-2 rounded-md border bg-background p-4">
          <p className="max-w-prose text-sm text-muted-foreground">
            Encrypted <span className="font-mono">tar | age</span> backups are
            created from the CLI. Run these in a terminal that has the kassiber
            command installed:
          </p>
          <CommandLine command="kassiber backup export" />
          <CommandLine command="kassiber backup import <file.age>" />
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <HardDrive className="size-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-semibold">Local database</h3>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <PathField
            id="settings-data-root"
            label="Data root"
            value={status?.data_root ?? null}
          />
          <PathField
            id="settings-db-path"
            label="Database"
            value={status?.database ?? null}
          />
        </div>
      </section>
    </div>
  );
}

export function DataStorageSettingsPanel({
  status,
  onOpenImports,
  onResetWelcome,
  onResetBook,
  resetBookDisabled,
  onDeleteBooks,
  deleteBooksDisabled,
}: {
  status: StatusData | null;
  onOpenImports: () => void;
  onResetWelcome: () => void;
  onResetBook: () => void;
  resetBookDisabled: boolean;
  onDeleteBooks: () => void;
  deleteBooksDisabled: boolean;
}) {
  return (
    <div className="space-y-6">
      <DataSettingsPanel status={status} onOpenImports={onOpenImports} />

      <section className="space-y-3 rounded-md border border-destructive/30 bg-destructive/5 p-4">
        <div className="space-y-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-destructive">
            <AlertTriangle className="size-4" aria-hidden="true" />
            Danger zone
          </h3>
          <p className="text-sm text-muted-foreground">
            Reset the Welcome gate, clear testing data, or delete the current
            local books set.
          </p>
        </div>
        <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">Reset Welcome state</p>
            <p className="text-sm text-muted-foreground">
              Clear only the local UI identity and return to onboarding.
              Encrypted data on disk is untouched.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={onResetWelcome}
          >
            Reset Welcome
          </Button>
        </div>
        <div className="flex flex-col gap-3 rounded-md border border-amber-500/30 bg-amber-500/5 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">Reset book data</p>
            <p className="text-sm text-muted-foreground">
              Keep wallet and backend connections, then clear synced
              transactions, journals, swaps, labels, attachments, and
              source-funds work. Shared fiat rates are optional.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            disabled={resetBookDisabled}
            onClick={onResetBook}
          >
            <RefreshCw className="mr-2 size-4" aria-hidden="true" />
            Reset book
          </Button>
        </div>
        <div className="flex flex-col gap-3 rounded-md border border-destructive/30 bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium text-destructive">
              Delete books set
            </p>
            <p className="text-sm text-muted-foreground">
              Remove the current books records from the local database.
            </p>
          </div>
          <Button
            type="button"
            variant="destructive"
            className="shrink-0"
            disabled={deleteBooksDisabled}
            onClick={onDeleteBooks}
          >
            Delete books
          </Button>
        </div>
      </section>
    </div>
  );
}
