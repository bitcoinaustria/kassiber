/**
 * Profiles / workspaces switcher screen.
 */

import { useState, type ComponentType, type SVGProps } from "react";
import {
  ArrowRight,
  BriefcaseBusiness,
  CheckCircle2,
  FolderPlus,
  Landmark,
  Plus,
  UserPlus,
  Users,
  Wallet,
} from "lucide-react";

import { useDaemon } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import type { ProfilesSnapshot, Profile, Workspace } from "@/mocks/profiles";

export function Profiles() {
  const { data, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
  );

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading profiles...
      </div>
    );
  }

  return <ProfilesView snapshot={data.data} />;
}

function ProfilesView({ snapshot }: { snapshot: ProfilesSnapshot }) {
  const [activeId, setActiveId] = useState(snapshot.activeProfileId);
  const workspaces = snapshot.workspaces;
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

  return (
    <div className="w-full space-y-4 p-4 sm:p-6">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 space-y-2">
          <h2 className="text-2xl font-semibold tracking-tight">
            Switch profile
          </h2>
          <p className="max-w-2xl text-sm text-muted-foreground">
            Profiles keep books, tax policy, accounts, and wallets separated.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline">
            <UserPlus className="size-4" aria-hidden="true" />
            Profile
          </Button>
          <Button type="button">
            <FolderPlus className="size-4" aria-hidden="true" />
            Workspace
          </Button>
        </div>
      </div>

      <div className="grid gap-3 md:grid-cols-3">
        <SummaryCard
          label="Workspaces"
          value={workspaces.length}
          detail="Identity scopes"
          icon={BriefcaseBusiness}
        />
        <SummaryCard
          label="Profiles"
          value={profileCount}
          detail="Tax policies"
          icon={Users}
        />
        <SummaryCard
          label="Wallets"
          value={walletCount}
          detail={`${accountCount} accounts`}
          icon={Wallet}
        />
      </div>

      <div className="space-y-4">
        {workspaces.map((workspace) => (
          <WorkspaceSection
            key={workspace.id}
            workspace={workspace}
            activeId={activeId}
            onPick={(profile) => setActiveId(profile.id)}
          />
        ))}
      </div>
    </div>
  );
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
  onPick: (profile: Profile) => void;
}

function WorkspaceSection({
  workspace,
  activeId,
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
          <span className="rounded-md border bg-muted/40 px-2 py-1 text-xs text-muted-foreground">
            {workspace.profiles.length} profile
            {workspace.profiles.length === 1 ? "" : "s"}
          </span>
          <Button type="button" variant="outline" size="sm">
            <Plus className="size-4" aria-hidden="true" />
            Profile
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
        <button
          type="button"
          className="flex min-h-[178px] flex-col items-center justify-center gap-2 rounded-xl border border-dashed bg-muted/20 p-4 text-center transition-colors hover:bg-muted/40"
        >
          <Plus className="size-5 text-muted-foreground" aria-hidden="true" />
          <span className="text-sm font-medium">
            New profile in {workspace.name}
          </span>
          <span className="text-xs text-muted-foreground">
            Inherit workspace defaults
          </span>
        </button>
      </CardContent>
    </Card>
  );
}

interface ProfileCardProps {
  profile: Profile;
  isActive: boolean;
  onPick: () => void;
}

function ProfileCard({ profile, isActive, onPick }: ProfileCardProps) {
  return (
    <button
      type="button"
      onClick={onPick}
      className={cn(
        "flex min-h-[178px] flex-col justify-between rounded-xl border p-4 text-left transition-colors hover:bg-muted/35",
        isActive ? "border-foreground bg-muted/45" : "bg-background",
      )}
    >
      <div className="space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="truncate font-medium">{profile.name}</p>
            <p className="mt-1 text-xs text-muted-foreground">
              {profile.role} · opened {profile.lastOpened}
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
          <p className="text-xs font-medium text-muted-foreground">Tax policy</p>
          <p className="mt-1 text-sm">{profile.taxPolicy}</p>
        </div>
      </div>

      <div className="mt-4 flex items-end justify-between gap-3">
        <div className="flex gap-4 text-sm">
          <span>
            <span className="block text-xs text-muted-foreground">Accounts</span>
            {profile.accounts}
          </span>
          <span>
            <span className="block text-xs text-muted-foreground">Wallets</span>
            {profile.wallets}
          </span>
        </div>
        <span
          className={cn(
            "inline-flex items-center gap-1 text-sm font-medium",
            isActive ? "text-foreground" : "text-muted-foreground",
          )}
        >
          {isActive ? "Current" : "Open"}
          <ArrowRight className="size-4" aria-hidden="true" />
        </span>
      </div>
    </button>
  );
}
