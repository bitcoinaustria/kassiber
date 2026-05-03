/**
 * Books / ledger switcher screen.
 */

import {
  useEffect,
  useState,
  type ComponentType,
  type SVGProps,
} from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowRight,
  BriefcaseBusiness,
  CheckCircle2,
  FolderPlus,
  Landmark,
  Plus,
  Users,
  Wallet,
} from "lucide-react";

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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { ProfilesSnapshot, Profile, Workspace } from "@/mocks/profiles";

export function Profiles() {
  const { data, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
  );

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading books...
      </div>
    );
  }

  return <ProfilesView snapshot={data.data} />;
}

function ProfilesView({ snapshot }: { snapshot: ProfilesSnapshot }) {
  const navigate = useNavigate();
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const createProfile = useDaemonMutation<{
    activeProfileId: string;
    activeWorkspaceId: string;
  }>("ui.profiles.create");
  const createWorkspace = useDaemonMutation<{
    activeProfileId: string;
    activeWorkspaceId: string;
  }>("ui.workspace.create");
  const [activeId, setActiveId] = useState(snapshot.activeProfileId);
  const [pendingSwitch, setPendingSwitch] =
    useState<PendingProfileSwitch | null>(null);
  const [profileWorkspace, setProfileWorkspace] = useState<Workspace | null>(
    null,
  );
  const [profileSource, setProfileSource] = useState<Profile | null>(null);
  const [profileName, setProfileName] = useState("");
  const [workspaceDialogOpen, setWorkspaceDialogOpen] = useState(false);
  const [workspaceName, setWorkspaceName] = useState("");
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [profileError, setProfileError] = useState<string | null>(null);
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

  const submitProfile = () => {
    if (!profileWorkspace || createProfile.isPending) return;
    const label = profileName.trim();
    if (!label) {
      setProfileError("Enter a books label.");
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
            error instanceof Error ? error.message : "Could not create books.",
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
            error instanceof Error ? error.message : "Could not switch books.",
          );
        },
      },
    );
  };

  const submitWorkspace = () => {
    if (createWorkspace.isPending) return;
    const label = workspaceName.trim();
    if (!label) {
      setWorkspaceError("Enter a ledger name.");
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
            error instanceof Error ? error.message : "Could not create ledger.",
          );
        },
      },
    );
  };

  return (
    <div className="w-full space-y-4 p-4 sm:p-6">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 space-y-2">
          <h2 className="text-2xl font-semibold tracking-tight">
            Switch books
          </h2>
          <p className="max-w-2xl text-sm text-muted-foreground">
            Books are separate accounting and tax scopes inside a ledger. Use
            them for private, business, or other activity that should not mix.
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
            New ledger
          </Button>
        </div>
      </div>

      <div className="grid gap-3 md:grid-cols-3">
        <SummaryCard
          label="Ledgers"
          value={workspaces.length}
          detail="Outer containers"
          icon={BriefcaseBusiness}
        />
        <SummaryCard
          label="Books"
          value={profileCount}
          detail="Accounting scopes"
          icon={Users}
        />
        <SummaryCard
          label="Wallets"
          value={walletCount}
          detail={`${accountCount} buckets`}
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
            onPick={(profile) => requestSwitch(workspace, profile)}
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
  onPick: (profile: Profile) => void;
}

function WorkspaceSection({
  workspace,
  activeId,
  onCreateProfile,
  onPick,
}: WorkspaceSectionProps) {
  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 border-b pb-5 md:flex-row md:items-center md:justify-between">
        <div className="space-y-1">
          <CardTitle className="flex items-center gap-2 text-lg">
            <Landmark className="size-4 text-muted-foreground" aria-hidden="true" />
            {workspace.name}
          </CardTitle>
          <CardDescription>
            {workspace.kind} · {workspace.currency} · {workspace.jurisdiction} ·
            since {workspace.created}
          </CardDescription>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onCreateProfile}
          >
            <Plus className="size-4" aria-hidden="true" />
            New books
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
}

function ProfileCard({
  profile,
  isActive,
  onPick,
}: ProfileCardProps) {
  return (
    <div
      className={cn(
        "flex min-h-[178px] flex-col justify-between rounded-xl border p-4 text-left transition-colors hover:bg-muted/35",
        isActive ? "border-foreground bg-muted/45" : "bg-background",
      )}
    >
      <button
        type="button"
        aria-current={isActive ? "true" : undefined}
        aria-label={
          isActive
            ? `Current books: ${profile.name}`
            : `Switch to ${profile.name} books`
        }
        onClick={onPick}
        className="flex flex-1 flex-col justify-between text-left"
      >
        <div className="space-y-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate font-medium">{profile.name}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                Opened {profile.lastOpened}
              </p>
            </div>
            {isActive && (
              <span className="inline-flex items-center gap-1 rounded-md border border-emerald-500/25 bg-emerald-500/10 px-2 py-1 text-xs font-medium text-emerald-700 dark:text-emerald-300">
                <CheckCircle2 className="size-3" aria-hidden="true" />
                Active
              </span>
            )}
          </div>

          <div className="rounded-lg border bg-muted/30 p-3">
            <p className="text-xs font-medium text-muted-foreground">
              Tax policy
            </p>
            <p className="mt-1 text-sm">{profile.taxPolicy}</p>
          </div>
        </div>

        <div className="mt-4 flex items-end justify-between gap-3">
          <div className="flex gap-4 text-sm">
            <span>
              <span className="block text-xs text-muted-foreground">
                Buckets
              </span>
              {profile.accounts}
            </span>
            <span>
              <span className="block text-xs text-muted-foreground">
                Wallets
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
            {isActive ? "Current" : "Switch"}
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
  const sourceValue = sourceProfile?.id ?? "__ledger_defaults__";
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
            <DialogTitle>New books</DialogTitle>
            <DialogDescription>
              {workspace && sourceProfile
                ? `Create separate books in ${workspace.name} from ${sourceProfile.name}'s tax settings.`
                : workspace
                  ? `Create separate books in ${workspace.name}.`
                  : "Create separate books using ledger defaults."}
            </DialogDescription>
          </DialogHeader>

          {workspace && (
            <div className="rounded-lg border bg-muted/25 p-3 text-sm">
              <p className="font-medium">{workspace.name}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                {workspace.kind} · {workspace.currency} ·{" "}
                {workspace.jurisdiction}
              </p>
            </div>
          )}

          {workspace && sourceOptions.length > 0 && (
            <div className="space-y-2">
              <Label htmlFor="books-source">Start from</Label>
              <Select
                value={sourceValue}
                disabled={isSubmitting}
                onValueChange={(value) => {
                  if (value === "__ledger_defaults__") {
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
                  <SelectItem value="__ledger_defaults__">
                    Ledger defaults
                  </SelectItem>
                  {sourceOptions.map((profile) => (
                    <SelectItem key={profile.id} value={profile.id}>
                      {profile.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs leading-5 text-muted-foreground">
                Copying settings only copies tax policy, currency, holding
                period, and lot selection. Wallets, buckets, and transactions
                stay in the original books.
              </p>
              {sourceProfile && (
                <p className="rounded-md border bg-muted/25 px-2 py-1 text-xs">
                  {sourceProfile.taxPolicy}
                </p>
              )}
            </div>
          )}

          <div className="space-y-2">
            <Label htmlFor="profile-name">Books label</Label>
            <Input
              id="profile-name"
              data-testid="profile-name-input"
              autoFocus
              aria-invalid={Boolean(errorMessage)}
              disabled={isSubmitting}
              placeholder="Business"
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
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              Create books
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
  const profile = pendingSwitch?.profile;
  const workspace = pendingSwitch?.workspace;

  return (
    <Dialog
      open={Boolean(pendingSwitch)}
      onOpenChange={(open) => !open && onCancel()}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Switch books?</DialogTitle>
          <DialogDescription>
            {currentProfile && profile
              ? `Switch from ${currentProfile.name} to ${profile.name}.`
              : "Switch to these books."}
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
                Tax policy
              </p>
              <p className="mt-1 text-sm">{profile.taxPolicy}</p>
            </div>
            <div className="mt-3 flex gap-4 text-sm">
              <span>
                <span className="block text-xs text-muted-foreground">
                  Buckets
                </span>
                {profile.accounts}
              </span>
              <span>
                <span className="block text-xs text-muted-foreground">Wallets</span>
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
            Cancel
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={isSubmitting}
            onClick={onSwitchHere}
          >
            Switch here
          </Button>
          <Button type="button" disabled={isSubmitting} onClick={onOpenOverview}>
            Switch and open Overview
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
            <DialogTitle>New ledger</DialogTitle>
            <DialogDescription>
              Create an empty ledger, then add its first books.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="workspace-name">Ledger name</Label>
            <Input
              id="workspace-name"
              data-testid="workspace-name-input"
              autoFocus
              aria-invalid={Boolean(errorMessage)}
              disabled={isSubmitting}
              placeholder="Personal books"
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
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              Create ledger
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
