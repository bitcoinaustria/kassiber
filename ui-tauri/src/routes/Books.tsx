/**
 * Books switcher screen.
 */

import {
  useEffect,
  useState,
  type ComponentType,
  type SVGProps,
} from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  ArrowRight,
  BriefcaseBusiness,
  CheckCircle2,
  Eye,
  FolderPlus,
  Landmark,
  Pencil,
  Plus,
  Users,
  Wallet,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { screenShellClassName } from "@/lib/screen-layout";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { gainsAlgorithmsFor } from "@/components/kb/Onboarding/constants";
import type { ProfilesSnapshot, Profile, Workspace } from "@/mocks/profiles";

const ACCOUNTING_METHOD_LABELS: Record<string, string> = {
  MOVING_AVERAGE_AT: "Moving average (Austrian)",
  FIFO: "FIFO (first in, first out)",
  LIFO: "LIFO (last in, first out)",
  HIFO: "HIFO (highest in, first out)",
  LOFO: "LOFO (lowest in, first out)",
};

const accountingMethodLabel = (method: string): string =>
  ACCOUNTING_METHOD_LABELS[method.toUpperCase()] ?? method;

export function Books() {
  const { data, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
  );

  if (isLoading || !data?.data) {
    return <ScreenSkeleton titleWidth="w-28" metricCount={3} />;
  }

  return <BooksView snapshot={data.data} />;
}

function BooksView({ snapshot }: { snapshot: ProfilesSnapshot }) {
  const { t } = useTranslation("onboarding");
  const navigate = useNavigate();
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const createProfile = useDaemonMutation<{
    activeProfileId: string;
    activeWorkspaceId: string;
  }>("ui.profiles.create");
  const renameProfile = useDaemonMutation<{
    profile: { id: string; name: string };
    workspace: { id: string };
  }>("ui.profiles.rename");
  const updateProfile = useDaemonMutation<{ id: string }>(
    "ui.profiles.update",
  );
  const createWorkspace = useDaemonMutation<{
    activeProfileId: string;
    activeWorkspaceId: string;
  }>("ui.workspace.create");
  const renameWorkspace = useDaemonMutation<{
    workspace: { id: string; name: string };
  }>("ui.workspace.rename");
  const [activeId, setActiveId] = useState(snapshot.activeProfileId);
  const [pendingSwitch, setPendingSwitch] =
    useState<PendingProfileSwitch | null>(null);
  const [profileWorkspace, setProfileWorkspace] = useState<Workspace | null>(
    null,
  );
  const [profileSource, setProfileSource] = useState<Profile | null>(null);
  const [profileName, setProfileName] = useState("");
  const [renameTarget, setRenameTarget] =
    useState<PendingProfileRename | null>(null);
  const [renameProfileName, setRenameProfileName] = useState("");
  const [renameProfileMethod, setRenameProfileMethod] = useState("");
  const [renameWorkspaceTarget, setRenameWorkspaceTarget] =
    useState<Workspace | null>(null);
  const [renameWorkspaceName, setRenameWorkspaceName] = useState("");
  const [workspaceDialogOpen, setWorkspaceDialogOpen] = useState(false);
  const [workspaceName, setWorkspaceName] = useState("");
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [profileError, setProfileError] = useState<string | null>(null);
  const [renameProfileError, setRenameProfileError] = useState<string | null>(
    null,
  );
  const [renameWorkspaceError, setRenameWorkspaceError] = useState<
    string | null
  >(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const workspaces = snapshot.workspaces;
  const activeProfile = findProfile(workspaces, activeId);
  const profileCount = workspaces.reduce((a, w) => a + w.profiles.length, 0);
  const walletCount = workspaces.reduce(
    (total, workspace) =>
      total +
      workspace.profiles.reduce((profileTotal, p) => profileTotal + p.wallets, 0),
    0,
  );
  const accountCount = workspaces.reduce(
    (total, workspace) =>
      total +
      workspace.profiles.reduce(
        (profileTotal, p) => profileTotal + p.accounts,
        0,
      ),
    0,
  );

  useEffect(() => {
    setActiveId(snapshot.activeProfileId);
  }, [snapshot.activeProfileId]);

  const requestSwitch = (workspace: Workspace, profile: Profile) => {
    if (profile.id === activeId) return;
    setSwitchError(null);
    setPendingSwitch({ workspace, profile });
  };

  const requestCreateProfile = (
    workspace: Workspace | null,
    sourceProfile: Profile | null = null,
  ) => {
    if (!workspace) return;
    setProfileError(null);
    setProfileName("");
    setProfileWorkspace(workspace);
    setProfileSource(sourceProfile);
  };

  const requestRenameProfile = (workspace: Workspace, profile: Profile) => {
    setRenameProfileError(null);
    setRenameProfileName(profile.name);
    setRenameProfileMethod(
      profile.gainsAlgorithm ??
        gainsAlgorithmsFor(profile.taxCountry ?? "generic")[0],
    );
    setRenameTarget({ workspace, profile });
  };

  const requestRenameWorkspace = (workspace: Workspace) => {
    setRenameWorkspaceError(null);
    setRenameWorkspaceName(workspace.name);
    setRenameWorkspaceTarget(workspace);
  };

  const submitProfile = () => {
    if (!profileWorkspace || createProfile.isPending) return;
    const label = profileName.trim();
    if (!label) {
      setProfileError(t("books.create.errorEmptyName"));
      return;
    }
    setProfileError(null);
    createProfile.mutate(
      {
        workspace_id: profileWorkspace.id,
        label,
        ...(profileSource ? { source_profile_id: profileSource.id } : {}),
      },
      {
        onSuccess: (response) => {
          setActiveId(response.data?.activeProfileId ?? "");
          setProfileWorkspace(null);
          setProfileSource(null);
          setProfileName("");
        },
        onError: (error) => {
          setProfileError(
            error instanceof Error
              ? error.message
              : t("books.create.errorGeneric"),
          );
        },
      },
    );
  };

  const confirmSwitch = (openOverview: boolean) => {
    if (!pendingSwitch || switchProfile.isPending) return;
    const nextProfile = pendingSwitch.profile;
    setSwitchError(null);
    switchProfile.mutate(
      { profile_id: nextProfile.id },
      {
        onSuccess: () => {
          setActiveId(nextProfile.id);
          setPendingSwitch(null);
          if (openOverview) {
            void navigate({ to: "/overview" });
          }
        },
        onError: (error) => {
          setSwitchError(
            error instanceof Error
              ? error.message
              : t("books.switch.errorGeneric"),
          );
        },
      },
    );
  };

  const submitRenameProfile = async () => {
    if (!renameTarget || renameProfile.isPending || updateProfile.isPending)
      return;
    const label = renameProfileName.trim();
    if (!label) {
      setRenameProfileError(t("books.renameProfile.errorEmptyName"));
      return;
    }
    setRenameProfileError(null);
    const profileId = renameTarget.profile.id;
    const nameChanged = label !== renameTarget.profile.name;
    const methodChanged =
      renameProfileMethod !== (renameTarget.profile.gainsAlgorithm ?? "");
    try {
      // Method first: update_profile enforces the Austrian method + invalidates
      // journals so reports recompute.
      if (methodChanged) {
        await updateProfile.mutateAsync({
          profile_id: profileId,
          gains_algorithm: renameProfileMethod,
        });
      }
      if (nameChanged) {
        await renameProfile.mutateAsync({ profile_id: profileId, label });
      }
      setRenameTarget(null);
      setRenameProfileName("");
      setRenameProfileMethod("");
    } catch (error) {
      setRenameProfileError(
        error instanceof Error
          ? error.message
          : t("books.renameProfile.errorGeneric"),
      );
    }
  };

  const submitRenameWorkspace = () => {
    if (!renameWorkspaceTarget || renameWorkspace.isPending) return;
    const label = renameWorkspaceName.trim();
    if (!label) {
      setRenameWorkspaceError(t("books.renameWorkspace.errorEmptyName"));
      return;
    }
    setRenameWorkspaceError(null);
    renameWorkspace.mutate(
      {
        workspace_id: renameWorkspaceTarget.id,
        label,
      },
      {
        onSuccess: () => {
          setRenameWorkspaceTarget(null);
          setRenameWorkspaceName("");
        },
        onError: (error) => {
          setRenameWorkspaceError(
            error instanceof Error
              ? error.message
              : t("books.renameWorkspace.errorGeneric"),
          );
        },
      },
    );
  };

  const submitWorkspace = () => {
    if (createWorkspace.isPending) return;
    const label = workspaceName.trim();
    if (!label) {
      setWorkspaceError(t("books.createWorkspace.errorEmptyName"));
      return;
    }
    setWorkspaceError(null);
    createWorkspace.mutate(
      { label },
      {
        onSuccess: () => {
          setActiveId("");
          setWorkspaceDialogOpen(false);
          setWorkspaceName("");
        },
        onError: (error) => {
          setWorkspaceError(
            error instanceof Error
              ? error.message
              : t("books.createWorkspace.errorGeneric"),
          );
        },
      },
    );
  };

  return (
    <div className={screenShellClassName}>
      <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 space-y-2">
          <h2 className="text-2xl font-semibold tracking-tight">
            {t("books.title")}
          </h2>
          <p className="max-w-2xl text-sm text-muted-foreground">
            {t("books.intro")}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            data-testid="create-workspace-button"
            onClick={() => {
              setWorkspaceError(null);
              setWorkspaceDialogOpen(true);
            }}
          >
            <FolderPlus className="size-4" aria-hidden="true" />
            {t("books.newBookSet")}
          </Button>
        </div>
      </div>

      <div className="grid gap-3 md:grid-cols-3">
        <SummaryCard
          label={t("books.summary.sets")}
          value={workspaces.length}
          detail={t("books.summary.setsDetail")}
          icon={BriefcaseBusiness}
        />
        <SummaryCard
          label={t("books.summary.books")}
          value={profileCount}
          detail={t("books.summary.booksDetail")}
          icon={Users}
        />
        <SummaryCard
          label={t("books.summary.wallets")}
          value={walletCount}
          detail={t("books.summary.walletsDetail", { count: accountCount })}
          icon={Wallet}
        />
      </div>

      <div className="space-y-4">
        {workspaces.map((workspace) => (
          <WorkspaceSection
            key={workspace.id}
            workspace={workspace}
            activeId={activeId}
            onCreateProfile={() => requestCreateProfile(workspace)}
            onOpenBirdsEye={() =>
              void navigate({
                to: "/books/$workspaceId/birds-eye",
                params: { workspaceId: workspace.id },
              })
            }
            onPick={(profile) => requestSwitch(workspace, profile)}
            onRename={(profile) => requestRenameProfile(workspace, profile)}
            onRenameWorkspace={() => requestRenameWorkspace(workspace)}
          />
        ))}
      </div>

      <ProfileSwitchDialog
        currentProfile={activeProfile?.profile ?? null}
        errorMessage={switchError}
        isSubmitting={switchProfile.isPending}
        pendingSwitch={pendingSwitch}
        onCancel={() => {
          setSwitchError(null);
          setPendingSwitch(null);
        }}
        onOpenOverview={() => confirmSwitch(true)}
        onSwitchHere={() => confirmSwitch(false)}
      />
      <CreateProfileDialog
        errorMessage={profileError}
        isSubmitting={createProfile.isPending}
        name={profileName}
        open={Boolean(profileWorkspace)}
        sourceProfile={profileSource}
        workspace={profileWorkspace}
        onNameChange={(value) => {
          setProfileName(value);
          if (profileError) setProfileError(null);
        }}
        onSourceProfileChange={(sourceProfile) => {
          setProfileSource(sourceProfile);
          if (profileError) setProfileError(null);
        }}
        onOpenChange={(open) => {
          if (createProfile.isPending) return;
          if (!open) {
            setProfileWorkspace(null);
            setProfileSource(null);
            setProfileName("");
            setProfileError(null);
          }
        }}
        onSubmit={submitProfile}
      />
      <RenameProfileDialog
        errorMessage={renameProfileError}
        isSubmitting={renameProfile.isPending || updateProfile.isPending}
        name={renameProfileName}
        method={renameProfileMethod}
        methodOptions={
          renameTarget
            ? Array.from(
                new Set<string>([
                  ...(renameTarget.profile.gainsAlgorithm
                    ? [renameTarget.profile.gainsAlgorithm]
                    : []),
                  ...gainsAlgorithmsFor(
                    renameTarget.profile.taxCountry ?? "generic",
                  ),
                ]),
              )
            : []
        }
        open={Boolean(renameTarget)}
        profile={renameTarget?.profile ?? null}
        workspace={renameTarget?.workspace ?? null}
        onNameChange={(value) => {
          setRenameProfileName(value);
          if (renameProfileError) setRenameProfileError(null);
        }}
        onMethodChange={(value) => {
          setRenameProfileMethod(value);
          if (renameProfileError) setRenameProfileError(null);
        }}
        onOpenChange={(open) => {
          if (renameProfile.isPending || updateProfile.isPending) return;
          if (!open) {
            setRenameTarget(null);
            setRenameProfileName("");
            setRenameProfileMethod("");
            setRenameProfileError(null);
          }
        }}
        onSubmit={submitRenameProfile}
      />
      <RenameWorkspaceDialog
        errorMessage={renameWorkspaceError}
        isSubmitting={renameWorkspace.isPending}
        name={renameWorkspaceName}
        open={Boolean(renameWorkspaceTarget)}
        workspace={renameWorkspaceTarget}
        onNameChange={(value) => {
          setRenameWorkspaceName(value);
          if (renameWorkspaceError) setRenameWorkspaceError(null);
        }}
        onOpenChange={(open) => {
          if (renameWorkspace.isPending) return;
          if (!open) {
            setRenameWorkspaceTarget(null);
            setRenameWorkspaceName("");
            setRenameWorkspaceError(null);
          }
        }}
        onSubmit={submitRenameWorkspace}
      />
      <CreateWorkspaceDialog
        errorMessage={workspaceError}
        isSubmitting={createWorkspace.isPending}
        name={workspaceName}
        open={workspaceDialogOpen}
        onNameChange={(value) => {
          setWorkspaceName(value);
          if (workspaceError) setWorkspaceError(null);
        }}
        onOpenChange={(open) => {
          if (createWorkspace.isPending) return;
          setWorkspaceDialogOpen(open);
          if (!open) {
            setWorkspaceError(null);
          }
        }}
        onSubmit={submitWorkspace}
      />
    </div>
  );
}

interface PendingProfileSwitch {
  workspace: Workspace;
  profile: Profile;
}

interface PendingProfileRename {
  workspace: Workspace;
  profile: Profile;
}

function findProfile(workspaces: Workspace[], profileId: string) {
  for (const workspace of workspaces) {
    const profile = workspace.profiles.find(
      (candidate) => candidate.id === profileId,
    );
    if (profile) {
      return { workspace, profile };
    }
  }
  return null;
}

function formatWorkspaceMeta(
  t: TFunction<"onboarding">,
  workspace: Workspace,
  options: { includeCreated?: boolean } = {},
) {
  const includeCreated = options.includeCreated ?? true;
  const parts = [
    workspace.currency,
    workspace.jurisdiction,
    includeCreated && workspace.created
      ? t("books.meta.since", { date: workspace.created })
      : null,
  ].filter(Boolean);
  return parts.join(" · ");
}

interface SummaryCardProps {
  label: string;
  value: number;
  detail: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
}

function SummaryCard({ label, value, detail, icon: Icon }: SummaryCardProps) {
  return (
    <Card className="gap-3 py-4">
      <CardContent className="flex items-center justify-between px-4">
        <div className="space-y-1">
          <p className="text-sm text-muted-foreground">{label}</p>
          <p className="text-2xl font-semibold tracking-tight">{value}</p>
          <p className="text-xs text-muted-foreground">{detail}</p>
        </div>
        <div className="flex size-10 items-center justify-center rounded-md border bg-muted/40">
          <Icon className="size-5 text-muted-foreground" aria-hidden="true" />
        </div>
      </CardContent>
    </Card>
  );
}

interface WorkspaceSectionProps {
  workspace: Workspace;
  activeId: string;
  onCreateProfile: () => void;
  onOpenBirdsEye: () => void;
  onPick: (profile: Profile) => void;
  onRename: (profile: Profile) => void;
  onRenameWorkspace: () => void;
}

export function WorkspaceSection({
  workspace,
  activeId,
  onCreateProfile,
  onOpenBirdsEye,
  onPick,
  onRename,
  onRenameWorkspace,
}: WorkspaceSectionProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 border-b pb-5 md:flex-row md:items-center md:justify-between">
        <div className="space-y-1">
          <CardTitle className="flex items-center gap-2 text-lg">
            <Landmark className="size-4 text-muted-foreground" aria-hidden="true" />
            {workspace.name}
          </CardTitle>
          <CardDescription>
            {formatWorkspaceMeta(t, workspace)}
          </CardDescription>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid={`birds-eye-${workspace.id}`}
            onClick={onOpenBirdsEye}
          >
            <Eye className="size-4" aria-hidden="true" />
            {t("books.workspace.overview")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="gap-2"
            onClick={onRenameWorkspace}
          >
            <Pencil className="size-4" aria-hidden="true" />
            {t("common:actions.edit")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onCreateProfile}
          >
            <Plus className="size-4" aria-hidden="true" />
            {t("books.workspace.newBook")}
          </Button>
        </div>
      </CardHeader>
      <CardContent className="grid gap-3 pt-5 md:grid-cols-2 xl:grid-cols-3">
        {workspace.profiles.map((profile) => (
          <ProfileCard
            key={profile.id}
            profile={profile}
            isActive={profile.id === activeId}
            onPick={() => onPick(profile)}
            onRename={() => onRename(profile)}
          />
        ))}
      </CardContent>
    </Card>
  );
}

interface ProfileCardProps {
  profile: Profile;
  isActive: boolean;
  onPick: () => void;
  onRename: () => void;
}

function ProfileCard({
  profile,
  isActive,
  onPick,
  onRename,
}: ProfileCardProps) {
  const { t } = useTranslation("onboarding");
  return (
    <div
      className={cn(
        "relative flex min-h-[178px] flex-col justify-between rounded-xl border p-4 text-left transition-colors hover:bg-muted/35",
        isActive ? "border-foreground bg-muted/45" : "bg-background",
      )}
    >
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="absolute top-3 right-3 z-10 size-8"
        aria-label={t("books.profileCard.editName", { name: profile.name })}
        onClick={onRename}
      >
        <Pencil className="size-3.5" aria-hidden="true" />
      </Button>
      <button
        type="button"
        aria-current={isActive ? "true" : undefined}
        aria-label={
          isActive
            ? t("books.profileCard.current", { name: profile.name })
            : t("books.profileCard.switchTo", { name: profile.name })
        }
        onClick={onPick}
        className="flex flex-1 flex-col justify-between text-left"
      >
        <div className="space-y-3">
          <div className="flex items-start justify-between gap-3 pr-10">
            <div className="min-w-0">
              <p className="truncate font-medium">{profile.name}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("books.profileCard.opened", { date: profile.lastOpened })}
              </p>
            </div>
            {isActive && (
              <span className="inline-flex items-center gap-1 rounded-md border border-emerald-500/25 bg-emerald-500/10 px-2 py-1 text-xs font-medium text-emerald-700 dark:text-emerald-300">
                <CheckCircle2 className="size-3" aria-hidden="true" />
                {t("books.profileCard.active")}
              </span>
            )}
          </div>

          <div className="rounded-lg border bg-muted/30 p-3">
            <p className="text-xs font-medium text-muted-foreground">
              {t("books.profileCard.taxPolicy")}
            </p>
            <p className="mt-1 text-sm">{profile.taxPolicy}</p>
          </div>
        </div>

        <div className="mt-4 flex items-end justify-between gap-3">
          <div className="flex gap-4 text-sm">
            <span>
              <span className="block text-xs text-muted-foreground">
                {t("books.profileCard.buckets")}
              </span>
              {profile.accounts}
            </span>
            <span>
              <span className="block text-xs text-muted-foreground">
                {t("books.profileCard.wallets")}
              </span>
              {profile.wallets}
            </span>
          </div>
          <span
            className={cn(
              "inline-flex items-center gap-1 text-sm font-medium",
              isActive ? "text-foreground" : "text-muted-foreground",
            )}
          >
            {isActive
              ? t("books.profileCard.currentLabel")
              : t("books.profileCard.switchLabel")}
            <ArrowRight className="size-4" aria-hidden="true" />
          </span>
        </div>
      </button>
    </div>
  );
}

interface CreateProfileDialogProps {
  errorMessage: string | null;
  isSubmitting: boolean;
  name: string;
  open: boolean;
  sourceProfile: Profile | null;
  workspace: Workspace | null;
  onNameChange: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  onSourceProfileChange: (sourceProfile: Profile | null) => void;
  onSubmit: () => void;
}

function CreateProfileDialog({
  errorMessage,
  isSubmitting,
  name,
  open,
  sourceProfile,
  workspace,
  onNameChange,
  onOpenChange,
  onSourceProfileChange,
  onSubmit,
}: CreateProfileDialogProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  const sourceValue = sourceProfile?.id ?? "__default_settings__";
  const sourceOptions = workspace?.profiles ?? [];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("books.create.title")}</DialogTitle>
            <DialogDescription>
              {workspace && sourceProfile
                ? t("books.create.descriptionFromSource", {
                    workspace: workspace.name,
                    source: sourceProfile.name,
                  })
                : workspace
                  ? t("books.create.descriptionInWorkspace", {
                      workspace: workspace.name,
                    })
                  : t("books.create.descriptionDefault")}
            </DialogDescription>
          </DialogHeader>

          {workspace && (
            <div className="rounded-lg border bg-muted/25 p-3 text-sm">
              <p className="font-medium">{workspace.name}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                {formatWorkspaceMeta(t, workspace, { includeCreated: false })}
              </p>
            </div>
          )}

          {workspace && sourceOptions.length > 0 && (
            <div className="space-y-2">
              <Label htmlFor="books-source">
                {t("books.create.startFrom")}
              </Label>
              <Select
                value={sourceValue}
                disabled={isSubmitting}
                onValueChange={(value) => {
                  if (value === "__default_settings__") {
                    onSourceProfileChange(null);
                    return;
                  }
                  onSourceProfileChange(
                    sourceOptions.find((profile) => profile.id === value) ??
                      null,
                  );
                }}
              >
                <SelectTrigger id="books-source" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__default_settings__">
                    {t("books.create.defaultSettings")}
                  </SelectItem>
                  {sourceOptions.map((profile) => (
                    <SelectItem key={profile.id} value={profile.id}>
                      {profile.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs leading-5 text-muted-foreground">
                {t("books.create.copyHint")}
              </p>
              {sourceProfile && (
                <p className="rounded-md border bg-muted/25 px-2 py-1 text-xs">
                  {sourceProfile.taxPolicy}
                </p>
              )}
            </div>
          )}

          <div className="space-y-2">
            <Label htmlFor="profile-name">{t("books.create.nameLabel")}</Label>
            <Input
              id="profile-name"
              data-testid="profile-name-input"
              autoFocus
              aria-invalid={Boolean(errorMessage)}
              disabled={isSubmitting}
              placeholder={t("books.create.namePlaceholder")}
              value={name}
              onChange={(event) => onNameChange(event.currentTarget.value)}
            />
          </div>

          {errorMessage && (
            <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </p>
          )}

          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {t("books.create.submit")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface RenameProfileDialogProps {
  errorMessage: string | null;
  isSubmitting: boolean;
  name: string;
  method: string;
  methodOptions: string[];
  open: boolean;
  profile: Profile | null;
  workspace: Workspace | null;
  onNameChange: (value: string) => void;
  onMethodChange: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: () => void;
}

function RenameProfileDialog({
  errorMessage,
  isSubmitting,
  name,
  method,
  methodOptions,
  open,
  profile,
  workspace,
  onNameChange,
  onMethodChange,
  onOpenChange,
  onSubmit,
}: RenameProfileDialogProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("books.renameProfile.title")}</DialogTitle>
            <DialogDescription>
              {t("books.renameProfile.description")}
            </DialogDescription>
          </DialogHeader>

          {profile && workspace && (
            <div className="rounded-lg border bg-muted/25 p-3 text-sm">
              <p className="font-medium">{profile.name}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                {workspace.name} · {workspace.currency} ·{" "}
                {workspace.jurisdiction}
              </p>
            </div>
          )}

          <div className="space-y-2">
            <Label htmlFor="rename-profile-name">
              {t("books.renameProfile.nameLabel")}
            </Label>
            <Input
              id="rename-profile-name"
              autoFocus
              aria-invalid={Boolean(errorMessage)}
              disabled={isSubmitting}
              value={name}
              onChange={(event) => onNameChange(event.currentTarget.value)}
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="rename-profile-method">Accounting method</Label>
            <Select
              value={method}
              disabled={isSubmitting || methodOptions.length <= 1}
              onValueChange={onMethodChange}
            >
              <SelectTrigger id="rename-profile-method">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {methodOptions.map((option) => (
                  <SelectItem key={option} value={option}>
                    {accountingMethodLabel(option)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {profile?.taxCountry === "at" ? (
              <p className="text-xs text-muted-foreground">
                Austrian books use the moving-average method (gleitender
                Durchschnittspreis); other methods aren&apos;t valid for Austria.
              </p>
            ) : null}
          </div>

          {errorMessage && (
            <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </p>
          )}

          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {t("books.renameProfile.submit")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface RenameWorkspaceDialogProps {
  errorMessage: string | null;
  isSubmitting: boolean;
  name: string;
  open: boolean;
  workspace: Workspace | null;
  onNameChange: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: () => void;
}

function RenameWorkspaceDialog({
  errorMessage,
  isSubmitting,
  name,
  open,
  workspace,
  onNameChange,
  onOpenChange,
  onSubmit,
}: RenameWorkspaceDialogProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("books.renameWorkspace.title")}</DialogTitle>
            <DialogDescription>
              {t("books.renameWorkspace.description")}
            </DialogDescription>
          </DialogHeader>

          {workspace && (
            <div className="rounded-lg border bg-muted/25 p-3 text-sm">
              <p className="font-medium">{workspace.name}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                {formatWorkspaceMeta(t, workspace, { includeCreated: false })}
              </p>
            </div>
          )}

          <div className="space-y-2">
            <Label htmlFor="rename-workspace-name">
              {t("books.renameWorkspace.nameLabel")}
            </Label>
            <Input
              id="rename-workspace-name"
              autoFocus
              aria-invalid={Boolean(errorMessage)}
              disabled={isSubmitting}
              value={name}
              onChange={(event) => onNameChange(event.currentTarget.value)}
            />
          </div>

          {errorMessage && (
            <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </p>
          )}

          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {t("books.renameWorkspace.submit")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface ProfileSwitchDialogProps {
  currentProfile: Profile | null;
  errorMessage: string | null;
  isSubmitting: boolean;
  pendingSwitch: PendingProfileSwitch | null;
  onCancel: () => void;
  onOpenOverview: () => void;
  onSwitchHere: () => void;
}

function ProfileSwitchDialog({
  currentProfile,
  errorMessage,
  isSubmitting,
  pendingSwitch,
  onCancel,
  onOpenOverview,
  onSwitchHere,
}: ProfileSwitchDialogProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  const profile = pendingSwitch?.profile;
  const workspace = pendingSwitch?.workspace;

  return (
    <Dialog
      open={Boolean(pendingSwitch)}
      onOpenChange={(open) => !open && onCancel()}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("books.switch.title")}</DialogTitle>
          <DialogDescription>
            {currentProfile && profile
              ? t("books.switch.descriptionFromTo", {
                  current: currentProfile.name,
                  next: profile.name,
                })
              : t("books.switch.descriptionGeneric")}
          </DialogDescription>
        </DialogHeader>

        {profile && workspace && (
          <div className="rounded-lg border bg-muted/25 p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="truncate font-medium">{profile.name}</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {workspace.name} · {workspace.currency} ·{" "}
                  {workspace.jurisdiction}
                </p>
              </div>
            </div>
            <div className="mt-3 rounded-md border bg-background/70 p-3">
              <p className="text-xs font-medium text-muted-foreground">
                {t("books.switch.taxPolicy")}
              </p>
              <p className="mt-1 text-sm">{profile.taxPolicy}</p>
            </div>
            <div className="mt-3 flex gap-4 text-sm">
              <span>
                <span className="block text-xs text-muted-foreground">
                  {t("books.switch.buckets")}
                </span>
                {profile.accounts}
              </span>
              <span>
                <span className="block text-xs text-muted-foreground">
                  {t("books.switch.wallets")}
                </span>
                {profile.wallets}
              </span>
            </div>
          </div>
        )}

        {errorMessage && (
          <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {errorMessage}
          </p>
        )}

        <DialogFooter className="gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={isSubmitting}
            onClick={onCancel}
          >
            {t("common:actions.cancel")}
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={isSubmitting}
            onClick={onSwitchHere}
          >
            {t("books.switch.switchHere")}
          </Button>
          <Button type="button" disabled={isSubmitting} onClick={onOpenOverview}>
            {t("books.switch.switchAndOpenOverview")}
            <ArrowRight className="size-4" aria-hidden="true" />
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface CreateWorkspaceDialogProps {
  errorMessage: string | null;
  isSubmitting: boolean;
  name: string;
  open: boolean;
  onNameChange: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: () => void;
}

function CreateWorkspaceDialog({
  errorMessage,
  isSubmitting,
  name,
  open,
  onNameChange,
  onOpenChange,
  onSubmit,
}: CreateWorkspaceDialogProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("books.createWorkspace.title")}</DialogTitle>
            <DialogDescription>
              {t("books.createWorkspace.description")}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="workspace-name">
              {t("books.createWorkspace.nameLabel")}
            </Label>
            <Input
              id="workspace-name"
              data-testid="workspace-name-input"
              autoFocus
              aria-invalid={Boolean(errorMessage)}
              disabled={isSubmitting}
              placeholder={t("books.createWorkspace.namePlaceholder")}
              value={name}
              onChange={(event) => onNameChange(event.currentTarget.value)}
            />
          </div>

          {errorMessage && (
            <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </p>
          )}

          <DialogFooter className="gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {t("books.createWorkspace.submit")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
