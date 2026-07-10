import * as React from "react";
import QRCode from "qrcode";
import QrScanner from "qr-scanner";
import {
  AlertTriangle,
  Check,
  Copy,
  FolderOpen,
  QrCode,
  RefreshCw,
  ShieldCheck,
  Trash2,
  UserPlus,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { pickFile } from "@/lib/filePicker";

interface SyncTransport {
  id: string;
  kind: "folder" | "webdav" | "s3";
  label: string;
  config: Record<string, string | null>;
  credentials_configured: boolean;
  last_push_at: string | null;
  last_pull_at: string | null;
  last_error_code: string | null;
  peers?: SyncPeer[];
}

interface SyncPeer {
  replica_id: string;
  member_name: string;
  device_label: string;
  last_head_seq: number;
  last_seen_at: string | null;
  status: "fresh" | "stale" | "never_seen";
}

interface SyncMember {
  id: string;
  display_name: string;
  role: "owner" | "editor" | "auditor";
  revoked_at: string | null;
  active_devices: number;
}

interface SyncDevice {
  id: string;
  member_id: string;
  member_name: string;
  label: string;
  local_device: number;
  revoked_at: string | null;
}

interface SyncConflict {
  id: string;
  entity_table: string;
  entity_key: string;
  field: string;
  first_value: unknown;
  second_value: unknown;
  first_event_id: string;
  second_event_id: string;
  events: Array<{ id: string; display_name: string; role: string; hlc: string }>;
}

interface SyncNotice {
  id: string;
  code: string;
  severity: "info" | "warning" | "blocking";
  created_at: string;
}

export interface SyncStatusData {
  configured: boolean;
  enabled: boolean;
  book_id?: string;
  local_member_id?: string;
  local_device_id?: string;
  local_replica_id?: string;
  members: number;
  devices: number;
  open_conflicts: number;
  version_vector: Record<string, number>;
  transports: SyncTransport[];
  notices: SyncNotice[];
  members_list: SyncMember[];
  devices_list: SyncDevice[];
  conflicts: SyncConflict[];
}

type TransportKind = "folder" | "webdav" | "s3";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function displayValue(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "string") return value || "—";
  return JSON.stringify(value, null, 2);
}

function QrPreview({ value, label }: { value: string; label: string }) {
  const [src, setSrc] = React.useState<string | null>(null);
  const [failed, setFailed] = React.useState(false);
  React.useEffect(() => {
    let active = true;
    setFailed(false);
    void QRCode.toDataURL(value, {
      errorCorrectionLevel: value.length > 2200 ? "L" : "M",
      margin: 2,
      width: 220,
      color: { dark: "#111111", light: "#ffffff" },
    })
      .then((next) => {
        if (active) setSrc(next);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, [value]);
  if (failed) return null;
  return src ? (
    <div className="w-fit rounded-md border bg-white p-2">
      <img src={src} alt={label} className="size-[220px]" />
    </div>
  ) : null;
}

function QrScanButton({ onScan }: { onScan: (value: string) => void }) {
  const { t } = useTranslation("settings");
  const inputRef = React.useRef<HTMLInputElement>(null);
  const [error, setError] = React.useState<string | null>(null);
  const scan = async (file: File) => {
    setError(null);
    try {
      const result = await QrScanner.scanImage(file, {
        returnDetailedScanResult: true,
      });
      onScan(typeof result === "string" ? result : result.data);
    } catch (scanError) {
      setError(errorMessage(scanError));
    }
  };
  return (
    <div className="space-y-1">
      <input
        ref={inputRef}
        className="sr-only"
        type="file"
        accept="image/*"
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (file) void scan(file);
          event.target.value = "";
        }}
      />
      <Button type="button" size="sm" variant="outline" onClick={() => inputRef.current?.click()}>
        <QrCode className="size-4" aria-hidden="true" />
        {t("sync.scanQr")}
      </Button>
      {error ? <p className="text-xs text-destructive">{t("sync.scanError")}</p> : null}
    </div>
  );
}

function CodeOutput({ value, label }: { value: string; label: string }) {
  const { t } = useTranslation("settings");
  const [copied, setCopied] = React.useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  };
  return (
    <div className="space-y-3 rounded-md border bg-muted/20 p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Label>{label}</Label>
        <Button type="button" size="sm" variant="outline" onClick={() => void copy()}>
          {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
          {copied ? t("sync.copied") : t("sync.copy")}
        </Button>
      </div>
      <Textarea value={value} readOnly className="min-h-24 font-mono text-xs" />
      <QrPreview value={value} label={label} />
    </div>
  );
}

export function SyncSettingsPanel({ encryptedWorkspace }: { encryptedWorkspace: boolean }) {
  const { t } = useTranslation("settings");
  const statusQuery = useDaemon<SyncStatusData>("ui.sync.status", undefined, {
    refetchOnMount: "always",
  });
  const enable = useDaemonMutation<SyncStatusData>("ui.sync.enable");
  const disable = useDaemonMutation<SyncStatusData>("ui.sync.disable");
  const configureTransport = useDaemonMutation<SyncTransport>("ui.sync.transports.configure");
  const deleteTransport = useDaemonMutation("ui.sync.transports.delete");
  const push = useDaemonMutation("ui.sync.push");
  const pull = useDaemonMutation("ui.sync.pull");
  const joinRequestMutation = useDaemonMutation<Record<string, unknown>>("ui.sync.join_request");
  const invite = useDaemonMutation<{ invitation: string }>("ui.sync.invite");
  const join = useDaemonMutation("ui.sync.join");
  const revokeMember = useDaemonMutation("ui.sync.members.revoke");
  const revokeDevice = useDaemonMutation("ui.sync.devices.revoke");
  const resolveConflict = useDaemonMutation("ui.sync.conflicts.resolve");

  const status = statusQuery.data?.data;
  const [error, setError] = React.useState<string | null>(null);
  const [memberName, setMemberName] = React.useState("");
  const [deviceLabel, setDeviceLabel] = React.useState("");
  const [transportKind, setTransportKind] = React.useState<TransportKind>("folder");
  const [transportLabel, setTransportLabel] = React.useState("");
  const [folderPath, setFolderPath] = React.useState("");
  const [url, setUrl] = React.useState("");
  const [username, setUsername] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [bucket, setBucket] = React.useState("");
  const [region, setRegion] = React.useState("us-east-1");
  const [prefix, setPrefix] = React.useState("");
  const [accessKey, setAccessKey] = React.useState("");
  const [secretKey, setSecretKey] = React.useState("");
  const [selectedTransport, setSelectedTransport] = React.useState("");
  const [joinMemberName, setJoinMemberName] = React.useState("");
  const [joinDeviceLabel, setJoinDeviceLabel] = React.useState("");
  const [joinRequest, setJoinRequest] = React.useState<Record<string, unknown> | null>(null);
  const [joinRequestCode, setJoinRequestCode] = React.useState("");
  const [invitationCode, setInvitationCode] = React.useState("");
  const [inviteRole, setInviteRole] = React.useState("editor");

  React.useEffect(() => {
    if (!selectedTransport && status?.transports[0]) {
      setSelectedTransport(status.transports[0].id);
    }
  }, [selectedTransport, status?.transports]);

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      await operation();
      await statusQuery.refetch();
    } catch (operationError) {
      setError(errorMessage(operationError));
    }
  };
  const localRole = status?.members_list.find((member) => member.id === status.local_member_id)?.role;
  const pending = [enable, disable, configureTransport, deleteTransport, push, pull, invite, join, revokeMember, revokeDevice, resolveConflict].some((mutation) => mutation.isPending);

  if (statusQuery.isLoading) {
    return <p className="text-sm text-muted-foreground">{t("sync.loading")}</p>;
  }
  if (statusQuery.error) {
    return <p className="text-sm text-destructive">{errorMessage(statusQuery.error)}</p>;
  }

  return (
    <div className="space-y-6">
      {error ? (
        <div role="alert" className="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      {!status?.configured ? (
        <div className="space-y-6">
        <section className="space-y-4 rounded-md border bg-background p-4">
          <div className="space-y-1">
            <h3 className="flex items-center gap-2 text-sm font-semibold">
              <ShieldCheck className="size-4 text-muted-foreground" />
              {t("sync.enableTitle")}
            </h3>
            <p className="text-sm text-muted-foreground">{t("sync.enableDescription")}</p>
          </div>
          {!encryptedWorkspace ? (
            <p className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
              {t("sync.encryptionRequired")}
            </p>
          ) : null}
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="sync-member-name">{t("sync.memberName")}</Label>
              <Input id="sync-member-name" value={memberName} onChange={(event) => setMemberName(event.target.value)} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="sync-device-label">{t("sync.deviceLabel")}</Label>
              <Input id="sync-device-label" value={deviceLabel} onChange={(event) => setDeviceLabel(event.target.value)} />
            </div>
          </div>
          <Button
            type="button"
            disabled={!encryptedWorkspace || !memberName.trim() || !deviceLabel.trim() || enable.isPending}
            onClick={() => void run(() => enable.mutateAsync({ member_name: memberName, device_label: deviceLabel }))}
          >
            {t("sync.enableButton")}
          </Button>
        </section>
        <section className="space-y-4 rounded-md border bg-background p-4">
          <div className="space-y-1">
            <h3 className="text-sm font-semibold">{t("sync.joinThisDevice")}</h3>
            <p className="text-sm text-muted-foreground">{t("sync.inviteDescription")}</p>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-2"><Label htmlFor="sync-unconfigured-member">{t("sync.memberName")}</Label><Input id="sync-unconfigured-member" value={joinMemberName} onChange={(event) => setJoinMemberName(event.target.value)} /></div>
            <div className="space-y-2"><Label htmlFor="sync-unconfigured-device">{t("sync.deviceLabel")}</Label><Input id="sync-unconfigured-device" value={joinDeviceLabel} onChange={(event) => setJoinDeviceLabel(event.target.value)} /></div>
          </div>
          <Button type="button" size="sm" variant="outline" disabled={!encryptedWorkspace || !joinMemberName || !joinDeviceLabel || joinRequestMutation.isPending} onClick={() => void run(async () => {
            const envelope = await joinRequestMutation.mutateAsync({ member_name: joinMemberName, device_label: joinDeviceLabel });
            setJoinRequest(envelope.data ?? null);
            setJoinRequestCode(JSON.stringify(envelope.data));
          })}><UserPlus className="size-4" />{t("sync.createJoinRequest")}</Button>
          {joinRequestCode ? <CodeOutput value={joinRequestCode} label={t("sync.joinRequestCode")} /> : null}
          <div className="space-y-2"><Label htmlFor="sync-unconfigured-invitation">{t("sync.invitationCode")}</Label><Textarea id="sync-unconfigured-invitation" value={invitationCode} onChange={(event) => setInvitationCode(event.target.value.trim())} className="min-h-24 font-mono text-xs" /></div>
          <QrScanButton onScan={setInvitationCode} />
          <Button type="button" size="sm" disabled={!joinRequest || !invitationCode || join.isPending} onClick={() => void run(() => join.mutateAsync({ request_id: joinRequest?.request_id, invitation: invitationCode }))}>{t("sync.joinButton")}</Button>
        </section>
        </div>
      ) : (
        <>
          <section className="space-y-3 rounded-md border bg-background p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <h3 className="text-sm font-semibold">{t("sync.statusTitle")}</h3>
                  <Badge variant={status.enabled ? "secondary" : "outline"}>
                    {status.enabled ? t("sync.enabled") : t("sync.paused")}
                  </Badge>
                  {status.open_conflicts > 0 ? (
                    <Badge variant="destructive">{t("sync.conflictCount", { count: status.open_conflicts })}</Badge>
                  ) : null}
                </div>
                <p className="text-sm text-muted-foreground">{t("sync.statusDescription", { members: status.members, devices: status.devices })}</p>
              </div>
              <Button type="button" size="sm" variant="outline" disabled={pending} onClick={() => void run(() => status.enabled ? disable.mutateAsync({}) : enable.mutateAsync({}))}>
                {status.enabled ? t("sync.pause") : t("sync.resume")}
              </Button>
            </div>
            {status.transports.length > 0 ? (
              <div className="flex flex-wrap items-center gap-2">
                <Select value={selectedTransport} onValueChange={setSelectedTransport}>
                  <SelectTrigger className="min-w-48"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {status.transports.map((transport) => <SelectItem key={transport.id} value={transport.id}>{transport.label}</SelectItem>)}
                  </SelectContent>
                </Select>
                <Button type="button" size="sm" variant="outline" disabled={!status.enabled || !selectedTransport || pending} onClick={() => void run(() => pull.mutateAsync({ transport_id: selectedTransport }))}>
                  <RefreshCw className="size-4" />{t("sync.pull")}
                </Button>
                <Button type="button" size="sm" disabled={!status.enabled || !selectedTransport || pending} onClick={() => void run(() => push.mutateAsync({ transport_id: selectedTransport }))}>
                  {t("sync.push")}
                </Button>
                {localRole === "owner" ? (
                  <Button type="button" size="sm" variant="outline" disabled={!status.enabled || !selectedTransport || pending} onClick={() => void run(() => push.mutateAsync({ transport_id: selectedTransport, snapshot: true }))}>
                    {t("sync.publishSnapshot")}
                  </Button>
                ) : null}
              </div>
            ) : null}
          </section>

          <section className="space-y-4">
            <div className="space-y-1">
              <h3 className="text-sm font-semibold">{t("sync.mailboxTitle")}</h3>
              <p className="text-sm text-muted-foreground">{t("sync.mailboxDescription")}</p>
            </div>
            <div className="space-y-3 rounded-md border bg-background p-4">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label>{t("sync.transportKind")}</Label>
                  <Select value={transportKind} onValueChange={(value) => setTransportKind(value as TransportKind)}>
                    <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="folder">{t("sync.folder")}</SelectItem>
                      <SelectItem value="webdav">WebDAV</SelectItem>
                      <SelectItem value="s3">S3</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="sync-transport-label">{t("sync.transportLabel")}</Label>
                  <Input id="sync-transport-label" value={transportLabel} onChange={(event) => setTransportLabel(event.target.value)} />
                </div>
              </div>
              {transportKind === "folder" ? (
                <div className="space-y-2">
                  <Label htmlFor="sync-folder-path">{t("sync.folderPath")}</Label>
                  <div className="flex gap-2">
                    <Input id="sync-folder-path" value={folderPath} onChange={(event) => setFolderPath(event.target.value)} />
                    <Button type="button" variant="outline" onClick={() => void pickFile({ directory: true, title: t("sync.chooseFolder") }).then((path) => path && setFolderPath(path))}>
                      <FolderOpen className="size-4" /><span className="sr-only">{t("sync.chooseFolder")}</span>
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-2 sm:col-span-2">
                    <Label htmlFor="sync-url">{transportKind === "webdav" ? t("sync.webdavUrl") : t("sync.s3Endpoint")}</Label>
                    <Input id="sync-url" value={url} onChange={(event) => setUrl(event.target.value)} />
                  </div>
                  {transportKind === "webdav" ? (
                    <>
                      <div className="space-y-2"><Label htmlFor="sync-username">{t("sync.username")}</Label><Input id="sync-username" value={username} onChange={(event) => setUsername(event.target.value)} /></div>
                      <div className="space-y-2"><Label htmlFor="sync-password">{t("sync.password")}</Label><Input id="sync-password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} /></div>
                    </>
                  ) : (
                    <>
                      <div className="space-y-2"><Label htmlFor="sync-bucket">{t("sync.bucket")}</Label><Input id="sync-bucket" value={bucket} onChange={(event) => setBucket(event.target.value)} /></div>
                      <div className="space-y-2"><Label htmlFor="sync-region">{t("sync.region")}</Label><Input id="sync-region" value={region} onChange={(event) => setRegion(event.target.value)} /></div>
                      <div className="space-y-2"><Label htmlFor="sync-prefix">{t("sync.prefix")}</Label><Input id="sync-prefix" value={prefix} onChange={(event) => setPrefix(event.target.value)} /></div>
                      <div className="space-y-2"><Label htmlFor="sync-access-key">{t("sync.accessKey")}</Label><Input id="sync-access-key" value={accessKey} onChange={(event) => setAccessKey(event.target.value)} /></div>
                      <div className="space-y-2 sm:col-span-2"><Label htmlFor="sync-secret-key">{t("sync.secretKey")}</Label><Input id="sync-secret-key" type="password" value={secretKey} onChange={(event) => setSecretKey(event.target.value)} /></div>
                    </>
                  )}
                </div>
              )}
              <Button
                type="button"
                disabled={!transportLabel.trim() || configureTransport.isPending}
                onClick={() => void run(async () => {
                  const config = transportKind === "folder"
                    ? { path: folderPath }
                    : transportKind === "webdav"
                      ? { url }
                      : { endpoint: url, bucket, region, prefix };
                  const credentials = transportKind === "webdav"
                    ? { username, password }
                    : transportKind === "s3"
                      ? { access_key: accessKey, secret_key: secretKey }
                      : {};
                  await configureTransport.mutateAsync({ kind: transportKind, label: transportLabel, config, credentials });
                  setPassword(""); setSecretKey(""); setTransportLabel("");
                })}
              >
                {t("sync.addTransport")}
              </Button>
            </div>
            {status.transports.map((transport) => (
              <div key={transport.id} className="flex flex-col gap-3 rounded-md border bg-background p-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="min-w-0">
                  <p className="text-sm font-medium">{transport.label}</p>
                  <p className="truncate text-xs text-muted-foreground">{transport.kind} · {transport.last_error_code ?? t("sync.ready")}</p>
                </div>
                <Button type="button" size="sm" variant="ghost" disabled={deleteTransport.isPending} onClick={() => void run(() => deleteTransport.mutateAsync({ transport_id: transport.id }))}>
                  <Trash2 className="size-4" /><span className="sr-only">{t("sync.removeTransport")}</span>
                </Button>
              </div>
            ))}
          </section>

          {status.notices.length > 0 ? (
            <section className="space-y-2 rounded-md border border-amber-500/30 bg-amber-500/5 p-4">
              <h3 className="flex items-center gap-2 text-sm font-semibold"><AlertTriangle className="size-4" />{t("sync.noticesTitle")}</h3>
              {status.notices.map((notice) => <p key={notice.id} className="font-mono text-xs">{notice.severity}: {notice.code}</p>)}
            </section>
          ) : null}

          <section className="space-y-4">
            <div className="space-y-1"><h3 className="text-sm font-semibold">{t("sync.inviteTitle")}</h3><p className="text-sm text-muted-foreground">{t("sync.inviteDescription")}</p></div>
            <div className="grid gap-4 xl:grid-cols-2">
              <div className="space-y-3 rounded-md border bg-background p-4">
                <h4 className="text-sm font-medium">{t("sync.joinThisDevice")}</h4>
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-2"><Label htmlFor="sync-join-member">{t("sync.memberName")}</Label><Input id="sync-join-member" value={joinMemberName} onChange={(event) => setJoinMemberName(event.target.value)} /></div>
                  <div className="space-y-2"><Label htmlFor="sync-join-device">{t("sync.deviceLabel")}</Label><Input id="sync-join-device" value={joinDeviceLabel} onChange={(event) => setJoinDeviceLabel(event.target.value)} /></div>
                </div>
                <Button type="button" size="sm" variant="outline" disabled={!joinMemberName || !joinDeviceLabel || joinRequestMutation.isPending} onClick={() => void run(async () => {
                  const envelope = await joinRequestMutation.mutateAsync({ member_name: joinMemberName, device_label: joinDeviceLabel });
                  setJoinRequest(envelope.data ?? null);
                  setJoinRequestCode(JSON.stringify(envelope.data));
                })}><UserPlus className="size-4" />{t("sync.createJoinRequest")}</Button>
                {joinRequestCode ? <CodeOutput value={joinRequestCode} label={t("sync.joinRequestCode")} /> : null}
                <div className="space-y-2"><Label htmlFor="sync-invitation-code">{t("sync.invitationCode")}</Label><Textarea id="sync-invitation-code" value={invitationCode} onChange={(event) => setInvitationCode(event.target.value.trim())} className="min-h-24 font-mono text-xs" /></div>
                <QrScanButton onScan={setInvitationCode} />
                <Button type="button" size="sm" disabled={!joinRequest || !invitationCode || join.isPending} onClick={() => void run(() => join.mutateAsync({ request_id: joinRequest?.request_id, invitation: invitationCode }))}>{t("sync.joinButton")}</Button>
              </div>

              <div className="space-y-3 rounded-md border bg-background p-4">
                <h4 className="text-sm font-medium">{t("sync.inviteSomeone")}</h4>
                <div className="space-y-2"><Label htmlFor="sync-request-code">{t("sync.joinRequestCode")}</Label><Textarea id="sync-request-code" value={joinRequestCode} onChange={(event) => setJoinRequestCode(event.target.value)} className="min-h-24 font-mono text-xs" /></div>
                <div className="flex flex-wrap items-center gap-2">
                  <QrScanButton onScan={setJoinRequestCode} />
                  <Select value={inviteRole} onValueChange={setInviteRole}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="editor">{t("sync.roleEditor")}</SelectItem><SelectItem value="auditor">{t("sync.roleAuditor")}</SelectItem><SelectItem value="owner">{t("sync.roleOwner")}</SelectItem></SelectContent></Select>
                  <Button type="button" size="sm" disabled={localRole !== "owner" || !joinRequestCode || invite.isPending} onClick={() => void run(async () => {
                    const request = JSON.parse(joinRequestCode) as Record<string, unknown>;
                    const envelope = await invite.mutateAsync({ join_request: request, role: inviteRole });
                    setInvitationCode(envelope.data?.invitation ?? "");
                  })}>{t("sync.createInvitation")}</Button>
                </div>
                {invitationCode ? <CodeOutput value={invitationCode} label={t("sync.invitationCode")} /> : null}
              </div>
            </div>
          </section>

          <section className="space-y-3">
            <h3 className="text-sm font-semibold">{t("sync.peopleDevicesTitle")}</h3>
            {status.members_list.map((member) => (
              <div key={member.id} className="flex flex-col gap-3 rounded-md border bg-background p-3 sm:flex-row sm:items-center sm:justify-between">
                <div><p className="text-sm font-medium">{member.display_name} <Badge variant="outline">{member.role}</Badge></p><p className="text-xs text-muted-foreground">{t("sync.activeDevices", { count: member.active_devices })}</p></div>
                {localRole === "owner" && member.id !== status.local_member_id && !member.revoked_at ? <Button type="button" size="sm" variant="outline" onClick={() => void run(() => revokeMember.mutateAsync({ member_id: member.id }))}>{t("sync.revoke")}</Button> : null}
              </div>
            ))}
            {status.devices_list.map((device) => (
              <div key={device.id} className="flex flex-col gap-3 rounded-md border bg-background p-3 sm:flex-row sm:items-center sm:justify-between">
                <div><p className="text-sm font-medium">{device.label} {device.local_device ? <Badge variant="secondary">{t("sync.thisDevice")}</Badge> : null}</p><p className="text-xs text-muted-foreground">{device.member_name}</p></div>
                {localRole === "owner" && !device.local_device && !device.revoked_at ? <Button type="button" size="sm" variant="ghost" onClick={() => void run(() => revokeDevice.mutateAsync({ device_id: device.id }))}>{t("sync.revoke")}</Button> : null}
              </div>
            ))}
          </section>

          <section className="space-y-3">
            <div><h3 className="text-sm font-semibold">{t("sync.conflictsTitle")}</h3><p className="text-sm text-muted-foreground">{t("sync.conflictsDescription")}</p></div>
            {status.conflicts.length === 0 ? <p className="rounded-md border bg-background p-3 text-sm text-muted-foreground">{t("sync.noConflicts")}</p> : status.conflicts.map((conflict) => (
              <div key={conflict.id} className="space-y-3 rounded-md border border-destructive/30 bg-destructive/5 p-4">
                <div><p className="text-sm font-medium">{conflict.entity_table} · {conflict.field}</p><p className="font-mono text-xs text-muted-foreground">{conflict.entity_key}</p></div>
                <div className="grid gap-3 sm:grid-cols-2">
                  {[[conflict.first_event_id, conflict.first_value], [conflict.second_event_id, conflict.second_value]].map(([eventId, value], index) => {
                    const author = conflict.events.find((item) => item.id === eventId);
                    return <div key={String(eventId)} className="space-y-2 rounded-md border bg-background p-3"><p className="text-xs text-muted-foreground">{author?.display_name ?? t("sync.unknownAuthor")}</p><pre className="overflow-auto whitespace-pre-wrap text-xs">{displayValue(value)}</pre><Button type="button" size="sm" disabled={resolveConflict.isPending} onClick={() => void run(() => resolveConflict.mutateAsync({ conflict_id: conflict.id, source_event_id: eventId }))}>{t("sync.keepChoice", { number: index + 1 })}</Button></div>;
                  })}
                </div>
              </div>
            ))}
          </section>
        </>
      )}
    </div>
  );
}
