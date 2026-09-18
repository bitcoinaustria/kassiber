import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { getTransport } from "@/daemon/transport";
import { isFilePickerAvailable, isFileSaveAvailable, pickFile, saveFile } from "@/lib/filePicker";
import { useUiStore } from "@/store/ui";

interface BackupPreview {
  token: string;
  target_data_root: string;
  target: { books: string[]; book_count: number };
  incoming: { books: string[]; book_count: number };
  attachments_files: number;
  replaces: string[];
  unavailable_secrets: boolean;
}

const backupErrorKeys = {
  invalid_backup: "backup.errors.invalid",
  missing_backup: "backup.errors.invalid",
  backup_passphrase_required: "backup.errors.passphrase",
  invalid_backup_destination: "backup.errors.destination",
  backup_export_failed: "backup.errors.destination",
  plaintext_database: "backup.errors.encryption",
  stale_backup_preview: "backup.errors.stale",
  backup_confirmation_required: "backup.errors.confirmation",
  unsafe_restore_target: "backup.errors.destination",
  project_in_use: "backup.errors.busy",
  backup_target_locked: "backup.errors.locked",
  restore_install_failed: "backup.errors.rolledBack",
  restore_rollback_failed: "backup.errors.recovery",
  restore_failed: "backup.errors.failedLocked",
} as const;

class BackupError extends Error {
  readonly code: string;
  readonly locked: boolean;
  readonly recoveryPath: string | null;
  constructor(code: string, details?: unknown) {
    super(code);
    this.code = code;
    const data = details && typeof details === "object" ? details as Record<string, unknown> : {};
    this.locked = data.locked === true || code === "backup_target_locked";
    this.recoveryPath = typeof data.recovery_path === "string" ? data.recovery_path : null;
  }
}

async function backupCall<T>(kind: string, args: Record<string, unknown>): Promise<T> {
  const result = await getTransport().invoke<T>({ kind, args });
  if (result.error || result.kind !== kind || !result.data) {
    throw new BackupError(result.error?.code ?? "backup_failed", result.error?.details);
  }
  return result.data;
}

export function BackupSettings() {
  const { t } = useTranslation("settings");
  const daemonSession = useUiStore((state) => state.daemonSession);
  const [mode, setMode] = useState<"export" | "restore" | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [secondPassphrase, setSecondPassphrase] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [preview, setPreview] = useState<BackupPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const token = useRef<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (token.current) void backupCall("ui.backup.cancel", { token: token.current }).catch(() => {});
    };
  }, []);

  function reset() {
    if (token.current) void backupCall("ui.backup.cancel", { token: token.current }).catch(() => {});
    token.current = null;
    setPreview(null);
    setPassphrase("");
    setSecondPassphrase("");
    setConfirmation("");
    setMode(null);
    setError(null);
  }

  async function run(action: "export" | "preview" | "apply") {
    setBusy(true);
    setError(null);
    setMessage(null);
    const current = () => mounted.current && useUiStore.getState().daemonSession === daemonSession;
    try {
      const filters = [{ name: "Kassiber", extensions: ["kassiber"] }];
      if (action === "export") {
        const path = await saveFile({ title: t("backup.export"), filters, defaultPath: "backup.kassiber" });
        if (!path || !current()) return;
        await backupCall("ui.backup.export", { path, backup_passphrase_secret: passphrase });
        if (current()) {
          reset();
          setMessage(t("backup.exported", { path }));
        }
      } else if (action === "preview") {
        const path = await pickFile({ title: t("backup.choose"), filters });
        if (!path || !current()) return;
        const result = await backupCall<BackupPreview>("ui.backup.preview", {
          path, backup_passphrase_secret: passphrase, database_passphrase_secret: secondPassphrase,
        });
        if (!current()) {
          await backupCall("ui.backup.cancel", { token: result.token });
          return;
        }
        token.current = result.token;
        setPreview(result);
      } else {
        const result = await backupCall<{ pre_restore_backup: string | null; warning: string | null }>("ui.backup.apply", {
          token: preview?.token, confirm: confirmation,
        });
        token.current = null;
        if (!current()) return;
        reset();
        useUiStore.getState().addNotification({ title: t("backup.restored"), body: result.warning ? t("backup.unlockWarning") : t("backup.recoverySaved", { path: result.pre_restore_backup ?? "" }), tone: result.warning ? "warning" : "success" });
        useUiStore.getState().bumpDaemonSession();
        window.dispatchEvent(new CustomEvent("kassiber:lock-app"));
      }
    } catch (failure) {
      if (current()) {
        const key = failure instanceof BackupError
          ? backupErrorKeys[failure.code as keyof typeof backupErrorKeys] : undefined;
        const text = key ? t(key) : t("backup.failed");
        if (failure instanceof BackupError && failure.locked) {
          token.current = null;
          reset();
          useUiStore.getState().addNotification({
            title: t("backup.failed"), tone: "error",
            body: failure.recoveryPath ? `${text} ${t("backup.recoverySaved", { path: failure.recoveryPath })}` : text,
          });
          useUiStore.getState().bumpDaemonSession();
          window.dispatchEvent(new CustomEvent("kassiber:lock-app"));
        } else {
          setError(text);
        }
      }
    } finally {
      if (mounted.current) {
        setBusy(false);
        setPassphrase("");
        setSecondPassphrase("");
      }
    }
  }

  return <div className="space-y-3 rounded-md border bg-background p-4">
    <p className="max-w-prose text-sm text-muted-foreground">{t("backup.description")}</p>
    <p className="text-sm text-muted-foreground">{t("backup.scope")}</p>
    {!mode ? <div className="flex flex-wrap gap-2">
      <Button variant="outline" disabled={!isFileSaveAvailable} onClick={() => { setMode("export"); setMessage(null); }}>{t("backup.export")}</Button>
      <Button variant="outline" disabled={!isFilePickerAvailable} onClick={() => { setMode("restore"); setMessage(null); }}>{t("backup.restore")}</Button>
    </div> : <div className="space-y-3">
      {!preview ? <>
        <div className="space-y-1"><Label htmlFor="backup-passphrase">{t("backup.passphrase")}</Label>
          <Input id="backup-passphrase" type="password" autoComplete="off" value={passphrase} disabled={busy} onChange={(event) => setPassphrase(event.target.value)} /></div>
        <div className="space-y-1"><Label htmlFor="backup-second-passphrase">{mode === "export" ? t("backup.confirmPassphrase") : t("backup.databasePassphrase")}</Label>
          <Input id="backup-second-passphrase" type="password" autoComplete="off" value={secondPassphrase} disabled={busy} onChange={(event) => setSecondPassphrase(event.target.value)} /></div>
        <Button disabled={busy || !passphrase || !secondPassphrase || (mode === "export" && passphrase !== secondPassphrase)} onClick={() => void run(mode === "export" ? "export" : "preview")}>
          {busy ? t("backup.working") : mode === "export" ? t("backup.export") : t("backup.preview")}
        </Button>
      </> : <>
        <p className="text-sm font-medium">{t("backup.replaceTarget", { count: preview.target.book_count })}</p>
        <p className="break-all font-mono text-xs">{preview.target_data_root}</p>
        <p className="text-sm">{preview.target.books.join(", ")}</p>
        <p className="text-sm">{t("backup.incoming", { count: preview.incoming.book_count, attachments: preview.attachments_files })}</p>
        <p className="text-sm">{preview.incoming.books.join(", ")}</p>
        <p className="text-sm text-muted-foreground">{t("backup.replaces", { paths: preview.replaces.join(", ") })}</p>
        <p className="text-sm text-muted-foreground">{t("backup.recovery")}</p>
        {preview.unavailable_secrets && <p className="text-sm text-amber-700 dark:text-amber-400">{t("backup.secretWarning")}</p>}
        <div className="space-y-1"><Label htmlFor="backup-confirmation">{t("backup.confirmRestore")}</Label>
          <Input id="backup-confirmation" value={confirmation} disabled={busy} autoComplete="off" onChange={(event) => setConfirmation(event.target.value)} /></div>
        <Button variant="destructive" disabled={busy || confirmation !== "RESTORE"} onClick={() => void run("apply")}>{busy ? t("backup.working") : t("backup.replace")}</Button>
      </>}
      <Button className="ml-2" variant="ghost" disabled={busy} onClick={reset}>{t("backup.cancel")}</Button>
    </div>}
    {!isFileSaveAvailable && <p className="text-sm text-muted-foreground">{t("backup.desktopOnly")}</p>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {message && <p role="status" className="break-all text-sm">{message}</p>}
  </div>;
}
