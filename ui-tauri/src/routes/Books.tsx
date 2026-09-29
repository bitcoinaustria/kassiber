/**
 * Books switcher screen.
 */

import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  Eye,
  FolderPlus,
  Landmark,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Search,
  Settings2,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageDescriptionClassName,
  pageHeaderClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import {
  GAINS_ALGORITHM_DEFAULTS,
  TAX_COUNTRIES,
  gainsAlgorithmsFor,
} from "@/components/kb/Onboarding/constants";
import type { TaxCountry } from "@/components/kb/Onboarding/types";
import type {
  CostBasisPoolScope,
  ProfilesSnapshot,
  Profile,
  Workspace,
} from "@/mocks/profiles";
import {
  poolScopesForProfileEdit,
  profileSettingsUpdatePayload,
} from "@/routes/books-profile-settings";

const ACCOUNTING_METHOD_LABEL_KEYS = {
  MOVING_AVERAGE_AT: "books.method.MOVING_AVERAGE_AT",
  MOVING_AVERAGE: "books.method.MOVING_AVERAGE",
  FIFO: "books.method.FIFO",
  LIFO: "books.method.LIFO",
  HIFO: "books.method.HIFO",
  LOFO: "books.method.LOFO",
} as const satisfies Record<string, string>;

const accountingMethodLabel = (
  method: string,
  t: TFunction<"onboarding">,
): string => {
  const key =
    ACCOUNTING_METHOD_LABEL_KEYS[
      method.toUpperCase() as keyof typeof ACCOUNTING_METHOD_LABEL_KEYS
    ];
  return key ? t(key) : method;
};

const TAX_COUNTRY_LABEL_KEYS = {
  at: "books.region.at",
  generic: "books.region.generic",
} as const satisfies Record<TaxCountry, string>;

const COST_BASIS_POOL_SCOPE_LABEL_KEYS = {
  global: "books.poolScope.global",
  wallet: "books.poolScope.wallet",
} as const satisfies Record<CostBasisPoolScope, string>;

const regionLabel = (
  country: TaxCountry,
  t: TFunction<"onboarding">,
): string => t(TAX_COUNTRY_LABEL_KEYS[country]);

const costBasisPoolScopeLabel = (
  scope: CostBasisPoolScope,
  t: TFunction<"onboarding">,
): string => t(COST_BASIS_POOL_SCOPE_LABEL_KEYS[scope]);

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
  // The book being opened: rows show its spinner and lock the others.
  const [pendingOpenId, setPendingOpenId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [profileWorkspace, setProfileWorkspace] = useState<Workspace | null>(
    null,
  );
  const [profileSource, setProfileSource] = useState<Profile | null>(null);
  const [profileName, setProfileName] = useState("");
  const [profileCountry, setProfileCountry] = useState<TaxCountry>("generic");
  const [profileMethod, setProfileMethod] = useState("");
  const [renameTarget, setRenameTarget] =
    useState<PendingProfileRename | null>(null);
  const [renameProfileName, setRenameProfileName] = useState("");
  const [renameProfileMethod, setRenameProfileMethod] = useState("");
  const [renameProfilePoolScope, setRenameProfilePoolScope] =
    useState<CostBasisPoolScope>("global");
  const [renameProfileCountry, setRenameProfileCountry] =
    useState<TaxCountry>("generic");
  const [regionSwitchConfirming, setRegionSwitchConfirming] = useState(false);
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
  const profileCount = workspaces.reduce((a, w) => a + w.profiles.length, 0);
  const visibleWorkspaces = filterWorkspaces(workspaces, query);

  useEffect(() => {
    setActiveId(snapshot.activeProfileId);
  }, [snapshot.activeProfileId]);

  // Opening a book is a switch, not an edit, so it happens at once and lands
  // on its Overview — the switcher in the title bar does the same without
  // leaving the page.
  const openBook = (profile: Profile) => {
    if (profile.id === activeId || switchProfile.isPending) return;
    setSwitchError(null);
    setPendingOpenId(profile.id);
    switchProfile.mutate(
      { profile_id: profile.id },
      {
        onSuccess: () => {
          setActiveId(profile.id);
          void navigate({ to: "/overview" });
        },
        onError: (error) => {
          setSwitchError(
            error instanceof Error
              ? error.message
              : t("books.switch.errorGeneric"),
          );
        },
        onSettled: () => setPendingOpenId(null),
      },
    );
  };

  const requestCreateProfile = (
    workspace: Workspace | null,
    sourceProfile: Profile | null = null,
  ) => {
    if (!workspace) return;
    setProfileError(null);
    setProfileName("");
    // Seed the region/method pickers from the workspace's existing default book
    // so "Default settings" matches what the daemon would otherwise inherit.
    const baseProfile = sourceProfile ?? workspace.profiles[0] ?? null;
    const baseCountry = baseProfile?.taxCountry ?? "generic";
    setProfileCountry(baseCountry);
    setProfileMethod(
      baseProfile?.gainsAlgorithm ?? gainsAlgorithmsFor(baseCountry)[0],
    );
    setProfileWorkspace(workspace);
    setProfileSource(sourceProfile);
  };

  const requestRenameProfile = (workspace: Workspace, profile: Profile) => {
    setRenameProfileError(null);
    setRegionSwitchConfirming(false);
    setRenameProfileName(profile.name);
    setRenameProfileCountry(profile.taxCountry ?? "generic");
    setRenameProfileMethod(
      profile.gainsAlgorithm ??
        gainsAlgorithmsFor(profile.taxCountry ?? "generic")[0],
    );
    setRenameProfilePoolScope(profile.costBasisPoolScope ?? "global");
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
        // Copy-from-source and explicit region/method are mutually exclusive:
        // copying inherits the source's settings; otherwise the picked region +
        // method apply.
        ...(profileSource
          ? { source_profile_id: profileSource.id }
          : {
              tax_country: profileCountry,
              gains_algorithm: profileMethod,
            }),
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
    const countryChanged =
      renameProfileCountry !== (renameTarget.profile.taxCountry ?? "generic");
    // A region switch resets the method and reprocesses journals — gate it
    // behind an explicit confirmation step.
    if (countryChanged && !regionSwitchConfirming) {
      setRegionSwitchConfirming(true);
      return;
    }
    try {
      // Method/region first: update_profile enforces the per-country method +
      // invalidates journals so reports recompute. A region switch must send a
      // region-valid method in the same call, since generic books reject the
      // Austrian method (and vice versa).
      const settingsUpdate = profileSettingsUpdatePayload(
        renameTarget.profile,
        renameProfileCountry,
        renameProfileMethod,
        renameProfilePoolScope,
      );
      if (settingsUpdate) {
        await updateProfile.mutateAsync(settingsUpdate);
      }
      if (nameChanged) {
        await renameProfile.mutateAsync({ profile_id: profileId, label });
      }
      setRenameTarget(null);
      setRenameProfileName("");
      setRenameProfileMethod("");
      setRenameProfilePoolScope("global");
      setRenameProfileCountry("generic");
      setRegionSwitchConfirming(false);
    } catch (error) {
      // Drop back to the form so the error is visible alongside the inputs.
      setRegionSwitchConfirming(false);
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
      <div className={pageHeaderClassName}>
        <p className={pageDescriptionClassName}>{t("books.intro")}</p>
        <div className={pageHeaderActionsClassName}>
          {profileCount > BOOK_FILTER_THRESHOLD ? (
            <div className="relative w-56">
              <Search
                className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("books.filter.placeholder")}
                aria-label={t("books.filter.placeholder")}
                className="h-8 pl-8"
              />
            </div>
          ) : null}
          <Button
            type="button"
            variant="outline"
            className={pageHeaderActionClassName}
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

      {switchError ? (
        <div
          role="alert"
          className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
        >
          {switchError}
        </div>
      ) : null}

      <div className="space-y-(--kb-page-gap)">
        {visibleWorkspaces.map((workspace) => (
          <WorkspaceSection
            key={workspace.id}
            workspace={workspace}
            activeId={activeId}
            pendingId={pendingOpenId}
            onCreateProfile={() => requestCreateProfile(workspace)}
            onOpenBirdsEye={() =>
              void navigate({
                to: "/books/$workspaceId/birds-eye",
                params: { workspaceId: workspace.id },
              })
            }
            onPick={openBook}
            onRename={(profile) => requestRenameProfile(workspace, profile)}
            onRenameWorkspace={() => requestRenameWorkspace(workspace)}
          />
        ))}
        {visibleWorkspaces.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            {t("books.filter.noMatches")}
          </p>
        ) : null}
      </div>

      <CreateProfileDialog
        errorMessage={profileError}
        isSubmitting={createProfile.isPending}
        name={profileName}
        country={profileCountry}
        method={profileMethod}
        methodOptions={gainsAlgorithmsFor(profileCountry)}
        open={Boolean(profileWorkspace)}
        sourceProfile={profileSource}
        workspace={profileWorkspace}
        onNameChange={(value) => {
          setProfileName(value);
          if (profileError) setProfileError(null);
        }}
        onCountryChange={(value) => {
          setProfileCountry(value);
          // Reset the method to the region's default; the previous method may be
          // invalid for the new region.
          setProfileMethod(GAINS_ALGORITHM_DEFAULTS[value]);
          if (profileError) setProfileError(null);
        }}
        onMethodChange={(value) => {
          // See RenameProfileDialog: ignore radix's spurious empty fire on a
          // region switch so the region default survives.
          if (!value) return;
          setProfileMethod(value);
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
        country={renameProfileCountry}
        method={renameProfileMethod}
        poolScope={renameProfilePoolScope}
        poolScopeOptions={
          renameTarget
            ? poolScopesForProfileEdit(renameTarget.profile, renameProfileCountry)
            : ["global"]
        }
        methodOptions={Array.from(
          new Set<string>([
            // Keep the stored method visible while still on the original region,
            // even if it isn't in the standard list (e.g. a legacy AT-on-FIFO
            // book). After a region switch the method is reset to that region's
            // default, so only the new region's methods apply.
            ...(renameTarget &&
            renameProfileCountry === (renameTarget.profile.taxCountry ?? "generic") &&
            renameTarget.profile.gainsAlgorithm
              ? [renameTarget.profile.gainsAlgorithm]
              : []),
            ...gainsAlgorithmsFor(renameProfileCountry),
          ]),
        )}
        confirmingRegionSwitch={regionSwitchConfirming}
        open={Boolean(renameTarget)}
        profile={renameTarget?.profile ?? null}
        workspace={renameTarget?.workspace ?? null}
        onNameChange={(value) => {
          setRenameProfileName(value);
          if (renameProfileError) setRenameProfileError(null);
        }}
        onCountryChange={(value) => {
          setRenameProfileCountry(value);
          // Reset the method to the new region's default — the previous method
          // may be invalid there, and update_profile would reject it.
          setRenameProfileMethod(GAINS_ALGORITHM_DEFAULTS[value]);
          setRenameProfilePoolScope("global");
          if (renameProfileError) setRenameProfileError(null);
        }}
        onMethodChange={(value) => {
          // A region switch changes the method value AND the option set in the
          // same render; radix Select then fires a spurious onValueChange("")
          // because the freshly-set value isn't in its (old) collection yet.
          // Ignore that empty so the region default isn't clobbered to blank.
          if (!value) return;
          setRenameProfileMethod(value);
          if (renameProfileError) setRenameProfileError(null);
        }}
        onPoolScopeChange={(value) => {
          setRenameProfilePoolScope(value);
          if (renameProfileError) setRenameProfileError(null);
        }}
        onCancelRegionSwitch={() => setRegionSwitchConfirming(false)}
        onOpenChange={(open) => {
          if (renameProfile.isPending || updateProfile.isPending) return;
          if (!open) {
            setRenameTarget(null);
            setRenameProfileName("");
            setRenameProfileMethod("");
            setRenameProfilePoolScope("global");
            setRenameProfileCountry("generic");
            setRegionSwitchConfirming(false);
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

interface PendingProfileRename {
  workspace: Workspace;
  profile: Profile;
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

const BOOK_FILTER_THRESHOLD = 6;

/**
 * Books whose name or tax policy match, under their set; a set whose own name
 * matches keeps all of its books.
 */
function filterWorkspaces(workspaces: Workspace[], query: string): Workspace[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return workspaces;
  return workspaces
    .map((workspace) =>
      workspace.name.toLowerCase().includes(needle)
        ? workspace
        : {
            ...workspace,
            profiles: workspace.profiles.filter((profile) =>
              `${profile.name} ${profile.taxPolicy}`.toLowerCase().includes(needle),
            ),
          },
    )
    .filter((workspace) => workspace.profiles.length > 0);
}

function bookInitials(name: string) {
  return (
    name
      .split(/\s+/)
      .map((part) => part[0])
      .join("")
      .slice(0, 2)
      .toUpperCase() || "?"
  );
}

interface WorkspaceSectionProps {
  workspace: Workspace;
  activeId: string;
  pendingId?: string | null;
  onCreateProfile: () => void;
  onOpenBirdsEye: () => void;
  onPick: (profile: Profile) => void;
  onRename: (profile: Profile) => void;
  onRenameWorkspace: () => void;
}

/**
 * One book set: a header with its identity and set-level actions, then its
 * books as rows — a list to scan and act on, not a wall of tiles.
 */
export function WorkspaceSection({
  workspace,
  activeId,
  pendingId = null,
  onCreateProfile,
  onOpenBirdsEye,
  onPick,
  onRename,
  onRenameWorkspace,
}: WorkspaceSectionProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <section className="kb-surface overflow-hidden" aria-label={workspace.name}>
      <header className="flex items-center gap-3 border-b px-(--kb-card-padding) py-3">
        <span
          className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
          aria-hidden="true"
        >
          <Landmark className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold">{workspace.name}</h2>
          <p className="truncate text-xs text-muted-foreground">
            {formatWorkspaceMeta(t, workspace)}
          </p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          data-testid={`birds-eye-${workspace.id}`}
          onClick={onOpenBirdsEye}
        >
          <Eye className="size-4" aria-hidden="true" />
          {t("books.workspace.overview")}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-8"
              aria-label={t("books.workspace.actions", { name: workspace.name })}
            >
              <MoreHorizontal className="size-4" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48">
            <DropdownMenuItem onSelect={onCreateProfile}>
              <Plus aria-hidden="true" />
              {t("books.workspace.newBook")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onRenameWorkspace}>
              <Pencil aria-hidden="true" />
              {t("books.workspace.rename")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </header>
      {workspace.profiles.length === 0 ? (
        <p className="px-(--kb-card-padding) py-4 text-sm text-muted-foreground">
          {t("books.workspace.empty")}
        </p>
      ) : (
        <ul className="divide-y">
          {workspace.profiles.map((profile) => (
            <BookRow
              key={profile.id}
              profile={profile}
              isActive={profile.id === activeId}
              isPending={profile.id === pendingId}
              locked={pendingId !== null}
              onPick={() => onPick(profile)}
              onRename={() => onRename(profile)}
            />
          ))}
        </ul>
      )}
      <button
        type="button"
        onClick={onCreateProfile}
        className="flex w-full items-center gap-2 border-t px-(--kb-card-padding) py-2.5 text-left text-sm text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:bg-muted/40 focus-visible:outline-none"
      >
        <Plus className="size-4" aria-hidden="true" />
        {t("books.workspace.newBookIn", { name: workspace.name })}
      </button>
    </section>
  );
}

interface BookRowProps {
  profile: Profile;
  isActive: boolean;
  isPending: boolean;
  locked: boolean;
  onPick: () => void;
  onRename: () => void;
}

function BookRow({
  profile,
  isActive,
  isPending,
  locked,
  onPick,
  onRename,
}: BookRowProps) {
  const { t } = useTranslation("onboarding");
  return (
    <li
      className={cn(
        "flex items-center gap-3 px-(--kb-card-padding) py-3",
        isActive && "bg-muted/40",
      )}
      aria-current={isActive ? "true" : undefined}
    >
      <span
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-lg text-xs font-semibold",
          isActive
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-muted-foreground",
        )}
        aria-hidden="true"
      >
        {bookInitials(profile.name)}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-medium">{profile.name}</span>
          {isActive ? (
            <Badge variant="secondary" className="shrink-0">
              {t("books.profileCard.currentLabel")}
            </Badge>
          ) : null}
        </div>
        <p className="truncate text-xs text-muted-foreground">
          {profile.taxPolicy}
        </p>
      </div>
      <dl className="hidden shrink-0 grid-cols-[4.5rem_4.5rem_7rem] gap-x-4 text-xs md:grid">
        <div>
          <dt className="text-muted-foreground">{t("books.profileCard.wallets")}</dt>
          <dd className="tabular-nums">{profile.wallets}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">{t("books.profileCard.buckets")}</dt>
          <dd className="tabular-nums">{profile.accounts}</dd>
        </div>
        <div className="min-w-0">
          <dt className="text-muted-foreground">{t("books.row.lastOpened")}</dt>
          <dd className="truncate">{profile.lastOpened}</dd>
        </div>
      </dl>
      <div className="flex w-28 shrink-0 items-center justify-end gap-1">
        {isActive ? null : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={locked}
            aria-label={t("books.profileCard.switchTo", { name: profile.name })}
            onClick={onPick}
          >
            {isPending ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : null}
            {t("books.row.open")}
          </Button>
        )}
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-8"
          aria-label={t("books.profileCard.settings", { name: profile.name })}
          title={t("books.profileCard.settings", { name: profile.name })}
          onClick={onRename}
        >
          <Settings2 className="size-4" aria-hidden="true" />
        </Button>
      </div>
    </li>
  );
}

interface CreateProfileDialogProps {
  errorMessage: string | null;
  isSubmitting: boolean;
  name: string;
  country: TaxCountry;
  method: string;
  methodOptions: string[];
  open: boolean;
  sourceProfile: Profile | null;
  workspace: Workspace | null;
  onNameChange: (value: string) => void;
  onCountryChange: (value: TaxCountry) => void;
  onMethodChange: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  onSourceProfileChange: (sourceProfile: Profile | null) => void;
  onSubmit: () => void;
}

function CreateProfileDialog({
  errorMessage,
  isSubmitting,
  name,
  country,
  method,
  methodOptions,
  open,
  sourceProfile,
  workspace,
  onNameChange,
  onCountryChange,
  onMethodChange,
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

          {sourceProfile ? null : (
            <>
              <div className="space-y-2">
                <Label htmlFor="create-profile-region">
                  {t("books.create.regionLabel")}
                </Label>
                <Select
                  value={country}
                  disabled={isSubmitting}
                  onValueChange={(value) =>
                    onCountryChange(value as TaxCountry)
                  }
                >
                  <SelectTrigger id="create-profile-region" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {TAX_COUNTRIES.map((option) => (
                      <SelectItem key={option} value={option}>
                        {regionLabel(option, t)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="create-profile-method">
                  {t("books.create.methodLabel")}
                </Label>
                <Select
                  value={method}
                  disabled={isSubmitting || methodOptions.length <= 1}
                  onValueChange={onMethodChange}
                >
                  <SelectTrigger id="create-profile-method" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {methodOptions.map((option) => (
                      <SelectItem key={option} value={option}>
                        {accountingMethodLabel(option, t)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {country === "at" ? (
                  <p className="text-xs text-muted-foreground">
                    {t("books.create.austrianMethodNote")}
                  </p>
                ) : null}
              </div>
            </>
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
  country: TaxCountry;
  method: string;
  methodOptions: string[];
  poolScope: CostBasisPoolScope;
  poolScopeOptions: CostBasisPoolScope[];
  confirmingRegionSwitch: boolean;
  open: boolean;
  profile: Profile | null;
  workspace: Workspace | null;
  onNameChange: (value: string) => void;
  onCountryChange: (value: TaxCountry) => void;
  onMethodChange: (value: string) => void;
  onPoolScopeChange: (value: CostBasisPoolScope) => void;
  onCancelRegionSwitch: () => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: () => void;
}

function RenameProfileDialog({
  errorMessage,
  isSubmitting,
  name,
  country,
  method,
  methodOptions,
  poolScope,
  poolScopeOptions,
  confirmingRegionSwitch,
  open,
  profile,
  workspace,
  onNameChange,
  onCountryChange,
  onMethodChange,
  onPoolScopeChange,
  onCancelRegionSwitch,
  onOpenChange,
  onSubmit,
}: RenameProfileDialogProps) {
  const { t } = useTranslation(["onboarding", "common"]);
  const fromCountry: TaxCountry = profile?.taxCountry ?? "generic";
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
            <DialogTitle>
              {confirmingRegionSwitch
                ? t("books.renameProfile.switchRegion.title")
                : t("books.renameProfile.title")}
            </DialogTitle>
            <DialogDescription>
              {confirmingRegionSwitch
                ? t("books.renameProfile.switchRegion.body", {
                    from: regionLabel(fromCountry, t),
                    to: regionLabel(country, t),
                    method: accountingMethodLabel(method, t),
                  })
                : t("books.renameProfile.description")}
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

          {confirmingRegionSwitch ? (
            <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-300">
              {t("books.renameProfile.switchRegion.warning")}
            </p>
          ) : (
            <>
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
                <Label htmlFor="rename-profile-region">
                  {t("books.renameProfile.regionLabel")}
                </Label>
                <Select
                  value={country}
                  disabled={isSubmitting}
                  onValueChange={(value) =>
                    onCountryChange(value as TaxCountry)
                  }
                >
                  <SelectTrigger id="rename-profile-region">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {TAX_COUNTRIES.map((option) => (
                      <SelectItem key={option} value={option}>
                        {regionLabel(option, t)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="rename-profile-method">
                  {t("books.renameProfile.methodLabel")}
                </Label>
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
                        {accountingMethodLabel(option, t)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {country === "at" ? (
                  <p className="text-xs text-muted-foreground">
                    {t("books.renameProfile.austrianMethodNote")}
                  </p>
                ) : null}
              </div>

              <div className="space-y-2">
                <Label htmlFor="rename-profile-pool-scope">
                  {t("books.renameProfile.poolScopeLabel")}
                </Label>
                <Select
                  value={poolScope}
                  disabled={isSubmitting || poolScopeOptions.length <= 1}
                  onValueChange={(value) =>
                    onPoolScopeChange(value as CostBasisPoolScope)
                  }
                >
                  <SelectTrigger id="rename-profile-pool-scope">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {poolScopeOptions.map((option) => (
                      <SelectItem key={option} value={option}>
                        {costBasisPoolScopeLabel(option, t)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {poolScopeOptions.length <= 1
                    ? t("books.renameProfile.poolScopeLockedNote")
                    : t("books.renameProfile.poolScopeNote")}
                </p>
              </div>
            </>
          )}

          {errorMessage && (
            <p className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </p>
          )}

          <DialogFooter className="gap-2">
            {confirmingRegionSwitch ? (
              <>
                <Button
                  type="button"
                  variant="outline"
                  disabled={isSubmitting}
                  onClick={onCancelRegionSwitch}
                >
                  {t("books.renameProfile.switchRegion.back")}
                </Button>
                <Button type="submit" disabled={isSubmitting}>
                  {t("books.renameProfile.switchRegion.confirm")}
                </Button>
              </>
            ) : (
              <>
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
              </>
            )}
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
