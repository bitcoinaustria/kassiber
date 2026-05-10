import { Link } from "@tanstack/react-router";
import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
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
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { cn } from "@/lib/utils";
import type { ProfilesSnapshot, Profile, Workspace } from "@/mocks/profiles";

interface BookSwitcherPopoverProps {
  open: boolean;
  onClose: () => void;
}

export function BookSwitcherPopover({
  open,
  onClose,
}: BookSwitcherPopoverProps) {
  const { data, error, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
    undefined,
    { enabled: open },
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="flex max-h-[min(82vh,720px)] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl">
        <DialogHeader className="border-b px-4 py-3 sm:px-5">
          <DialogTitle className="text-base">Switch books</DialogTitle>
          <DialogDescription>
            Pick the active book/profile for every screen in this window.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-5">
          {isLoading ? (
            <div className="flex min-h-[180px] items-center justify-center text-sm text-muted-foreground">
              <Loader2 className="mr-2 size-4 animate-spin" aria-hidden="true" />
              Loading books...
            </div>
          ) : error ? (
            <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
              {error instanceof Error
                ? error.message
                : "Could not load books."}
            </div>
          ) : data?.data ? (
            <SwitcherBody snapshot={data.data} onClose={onClose} />
          ) : (
            <div className="rounded-lg border bg-muted/35 p-3 text-sm text-muted-foreground">
              No books were found in this data root.
            </div>
          )}
        </div>

        <DialogFooter className="border-t bg-muted/25 px-4 py-3 sm:px-5">
          <Button variant="outline" size="sm" asChild>
            <Link to="/books" onClick={onClose}>
              Manage books
            </Link>
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface SwitcherBodyProps {
  snapshot: ProfilesSnapshot;
  onClose: () => void;
}

function SwitcherBody({ snapshot, onClose }: SwitcherBodyProps) {
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const [activeId, setActiveId] = useState(snapshot.activeProfileId);
  const [pendingId, setPendingId] = useState<string | null>(null);

  useEffect(() => {
    setActiveId(snapshot.activeProfileId);
  }, [snapshot.activeProfileId]);

  const handlePick = (profile: Profile) => {
    if (profile.id === activeId) {
      onClose();
      return;
    }
    setPendingId(profile.id);
    switchProfile.mutate(
      { profile_id: profile.id },
      {
        onSuccess: () => {
          setActiveId(profile.id);
          onClose();
        },
        onSettled: () => setPendingId(null),
      },
    );
  };

  return (
    <div className="space-y-4">
      {snapshot.workspaces.map((workspace) => (
        <WorkspaceBlock
          key={workspace.id}
          workspace={workspace}
          activeId={activeId}
          pendingId={pendingId}
          switching={switchProfile.isPending}
          onPick={handlePick}
        />
      ))}
      {switchProfile.error ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
          {switchProfile.error instanceof Error
            ? switchProfile.error.message
            : "Could not switch books."}
        </div>
      ) : null}
    </div>
  );
}

interface WorkspaceBlockProps {
  workspace: Workspace;
  activeId: string;
  pendingId: string | null;
  switching: boolean;
  onPick: (profile: Profile) => void;
}

function WorkspaceBlock({
  workspace,
  activeId,
  pendingId,
  switching,
  onPick,
}: WorkspaceBlockProps) {
  return (
    <Card className="gap-3 rounded-lg py-0">
      <CardHeader className="gap-2 px-3 py-3 sm:px-4">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <CardTitle className="min-w-0 truncate text-sm">
            {workspace.name}
          </CardTitle>
          <Badge variant="outline">{workspace.jurisdiction}</Badge>
          <Badge variant="secondary">
            {workspace.kind} · {workspace.currency}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="grid gap-2 px-3 pb-3 sm:grid-cols-2 sm:px-4">
        {workspace.profiles.map((profile) => (
          <ProfileOption
            key={profile.id}
            profile={profile}
            isActive={profile.id === activeId}
            isPending={profile.id === pendingId}
            disabled={switching}
            onPick={() => onPick(profile)}
          />
        ))}
      </CardContent>
    </Card>
  );
}

interface ProfileOptionProps {
  profile: Profile;
  isActive: boolean;
  isPending: boolean;
  disabled: boolean;
  onPick: () => void;
}

function ProfileOption({
  profile,
  isActive,
  isPending,
  disabled,
  onPick,
}: ProfileOptionProps) {
  return (
    <Button
      type="button"
      variant={isActive ? "secondary" : "outline"}
      className={cn(
        "relative h-auto min-h-[116px] items-stretch justify-start whitespace-normal rounded-lg px-3 py-3 text-left",
        isActive &&
          "border-primary bg-primary/15 text-foreground ring-2 ring-primary/35",
      )}
      disabled={disabled}
      aria-current={isActive ? "true" : undefined}
      onClick={onPick}
    >
      <span className="grid min-w-0 flex-1 grid-rows-[auto_auto_minmax(1.25rem,auto)] gap-2">
        <span className="flex min-w-0 items-start justify-between gap-2">
          <span className="min-w-0">
            <span className="block truncate font-medium">{profile.name}</span>
            <span className="mt-0.5 block text-xs text-muted-foreground">
              Opened {profile.lastOpened}
            </span>
          </span>
          {isPending ? (
            <Loader2
              className="mt-0.5 size-4 shrink-0 animate-spin text-muted-foreground"
              aria-hidden="true"
            />
          ) : null}
        </span>
        <span className="flex min-w-0 flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
          <span>{profile.accounts} buckets</span>
          <span>{profile.wallets} wallets</span>
        </span>
        <span className="line-clamp-2 min-w-0 text-xs text-muted-foreground">
          {profile.taxPolicy}
        </span>
      </span>
    </Button>
  );
}
