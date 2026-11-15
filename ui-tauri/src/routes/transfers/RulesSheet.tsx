/**
 * Linking rules and saved views of the suggestions, kept out of the way in a
 * sheet off the Transfers page's menu. A rule marks the suggestions it
 * matches and sets the kind and tax treatment their link records; the owner
 * still confirms each link. A saved view filters the suggestions.
 */
import { useState } from "react";
import { Loader2, Plus, Star, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import { useDaemon, useDaemonMutation } from "@/daemon/client";

import { kindKey } from "./linkModel";
import { ruleSentence, type Translate } from "./rulesModel";

const PAIR_KIND_OPTIONS = [
  "manual",
  "coinjoin",
  "whirlpool",
  "chain-swap",
  "peg-in",
  "peg-out",
  "reverse-submarine-swap",
  "submarine-swap",
  "swap-refund",
] as const;
const PAIR_POLICY_OPTIONS = ["carrying-value", "taxable"] as const;
const WALLET_KIND_OPTIONS = ["phoenix", "coreln", "lnd", "nwc", "descriptor", "xpub", "address", "custom"] as const;
type PairKind = (typeof PAIR_KIND_OPTIONS)[number];
type PairPolicy = (typeof PAIR_POLICY_OPTIONS)[number];

interface SavedView {
  id: string;
  surface: string;
  name: string;
  filter: {
    confidence?: string;
    method?: string;
    asset_pair?: string;
    route_pair?: string;
    [key: string]: unknown;
  };
}

interface SavedViewsEnvelope {
  views: SavedView[];
}

interface SwapRule {
  id: string;
  name: string | null;
  predicate: Record<string, unknown>;
  kind: PairKind;
  policy: PairPolicy;
  enabled: boolean;
}

interface RulesEnvelope {
  rules: SwapRule[];
}

interface CreateRulePayload {
  name: string | null;
  predicate: Record<string, unknown>;
  kind: PairKind;
  policy: PairPolicy;
  enabled: boolean;
}

interface CreateRuleDialogProps {
  open: boolean;
  onClose: () => void;
  onCreate: (payload: CreateRulePayload) => Promise<void>;
  isCreating: boolean;
}

function CreateRuleDialog({ open, onClose, onCreate, isCreating }: CreateRuleDialogProps) {
  const { t } = useTranslation(["review", "common"]);
  const [name, setName] = useState("");
  const [outAsset, setOutAsset] = useState("any");
  const [inAsset, setInAsset] = useState("any");
  const [outKind, setOutKind] = useState("any");
  const [inKind, setInKind] = useState("any");
  const [maxFeePct, setMaxFeePct] = useState("");
  const [minConfidence, setMinConfidence] = useState<"strong" | "exact">("strong");
  const [kind, setKind] = useState<PairKind>("submarine-swap");
  const [policy, setPolicy] = useState<PairPolicy>("carrying-value");

  const reset = () => {
    setName("");
    setOutAsset("any");
    setInAsset("any");
    setOutKind("any");
    setInKind("any");
    setMaxFeePct("");
    setMinConfidence("strong");
    setKind("submarine-swap");
    setPolicy("carrying-value");
  };

  const submit = async () => {
    const predicate: Record<string, unknown> = {};
    if (outAsset !== "any") predicate.out_asset = outAsset;
    if (inAsset !== "any") predicate.in_asset = inAsset;
    if (outKind !== "any") predicate.out_wallet_kind = outKind;
    if (inKind !== "any") predicate.in_wallet_kind = inKind;
    if (maxFeePct.trim()) {
      const parsed = Number.parseFloat(maxFeePct.trim());
      if (Number.isFinite(parsed)) predicate.max_fee_pct = parsed;
    }
    predicate.min_confidence = minConfidence;
    await onCreate({
      name: name.trim() || null,
      predicate,
      kind,
      policy,
      enabled: true,
    });
    reset();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value) {
          reset();
          onClose();
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("swap.createRule.title")}</DialogTitle>
          <DialogDescription>
            {t("swap.createRule.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <div className="col-span-2 space-y-1">
            <Label htmlFor="rule-name">{t("swap.createRule.nameLabel")}</Label>
            <Input
              id="rule-name"
              placeholder={t("swap.createRule.namePlaceholder")}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <RulePredicateAssetField
            label={t("swap.createRule.outAsset")}
            value={outAsset}
            onChange={setOutAsset}
          />
          <RulePredicateAssetField
            label={t("swap.createRule.inAsset")}
            value={inAsset}
            onChange={setInAsset}
          />
          <RulePredicateKindField
            label={t("swap.createRule.outWalletKind")}
            value={outKind}
            onChange={setOutKind}
          />
          <RulePredicateKindField
            label={t("swap.createRule.inWalletKind")}
            value={inKind}
            onChange={setInKind}
          />
          <div className="space-y-1">
            <Label htmlFor="max-fee">{t("swap.createRule.maxFee")}</Label>
            <Input
              id="max-fee"
              placeholder={t("swap.createRule.maxFeePlaceholder")}
              value={maxFeePct}
              onChange={(e) => setMaxFeePct(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label>{t("swap.createRule.minConfidence")}</Label>
            <Select
              value={minConfidence}
              onValueChange={(v) => setMinConfidence(v as "strong" | "exact")}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="strong">{t("swap.createRule.minStrong")}</SelectItem>
                <SelectItem value="exact">{t("swap.createRule.minExact")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>{t("swap.createRule.kind")}</Label>
            <Select value={kind} onValueChange={(v) => setKind(v as PairKind)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PAIR_KIND_OPTIONS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {t(`transfersPage.kind.${kindKey(option)}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>{t("swap.createRule.policy")}</Label>
            <Select value={policy} onValueChange={(v) => setPolicy(v as PairPolicy)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PAIR_POLICY_OPTIONS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {t(`transfersPage.policy.${option === "taxable" ? "taxable" : "carrying"}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => {
              reset();
              onClose();
            }}
          >
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={() => void submit()} disabled={isCreating}>
            {isCreating ? <Loader2 className="size-4 animate-spin" /> : null}
            <span className="ml-1">{t("swap.createRule.createButton")}</span>
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface RuleFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
}

function RulePredicateAssetField({ label, value, onChange }: RuleFieldProps) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-1">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="any">{t("swap.createRule.any")}</SelectItem>
          <SelectItem value="BTC">{t("swap.createRule.assetBtc")}</SelectItem>
          <SelectItem value="LBTC">{t("swap.createRule.assetLbtc")}</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}

function RulePredicateKindField({ label, value, onChange }: RuleFieldProps) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-1">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="any">{t("swap.createRule.any")}</SelectItem>
          {WALLET_KIND_OPTIONS.map((option) => (
            <SelectItem key={option} value={option}>
              {t(`swap.createRule.walletKind.${option}`)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

export function RulesAndViewsSheet({
  open,
  onOpenChange,
  onOpenView,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpenView: (view: SavedView) => void;
}) {
  const { t } = useTranslation(["review", "common"]);
  const rulesQuery = useDaemon<RulesEnvelope>("ui.transfers.rules.list", undefined, { enabled: open });
  const transferViewsQuery = useDaemon<SavedViewsEnvelope>(
    "ui.saved_views.list",
    { surface: "transfer_candidates" },
    { enabled: open },
  );
  const swapViewsQuery = useDaemon<SavedViewsEnvelope>(
    "ui.saved_views.list",
    { surface: "swap_candidates" },
    { enabled: open },
  );
  const ruleCreate = useDaemonMutation<SwapRule>("ui.transfers.rules.create");
  const ruleDelete = useDaemonMutation<unknown>("ui.transfers.rules.delete");
  const ruleSetEnabled = useDaemonMutation<SwapRule>("ui.transfers.rules.set_enabled");
  const savedViewDelete = useDaemonMutation<unknown>("ui.saved_views.delete");
  const [createRuleOpen, setCreateRuleOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rules = rulesQuery.data?.data?.rules ?? [];
  const views = [
    ...(transferViewsQuery.data?.data?.views ?? []),
    ...(swapViewsQuery.data?.data?.views ?? []),
  ];

  const run = async (action: () => Promise<unknown>, refetch: () => unknown) => {
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("swap.rules.failed"));
    } finally {
      void refetch();
    }
  };

  return (
    <>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent className="w-full overflow-y-auto overscroll-none sm:max-w-lg">
          <SheetHeader>
            <SheetTitle>{t("swap.rules.sheetTitle")}</SheetTitle>
            <SheetDescription>{t("swap.rules.sheetDescription")}</SheetDescription>
          </SheetHeader>
          <div className="space-y-6 px-4 pb-6">
            <section className="space-y-2" aria-labelledby="linking-rules-title">
              <div className="flex items-center justify-between gap-2">
                <h3 id="linking-rules-title" className="text-sm font-medium">
                  {t("swap.rules.title")}
                </h3>
                <Button size="sm" variant="outline" className="h-8 gap-1.5" onClick={() => setCreateRuleOpen(true)}>
                  <Plus className="size-3.5" aria-hidden="true" />
                  {t("swap.rules.new")}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">{t("swap.rules.explain")}</p>
              {rulesQuery.isLoading ? (
                <p className="text-sm text-muted-foreground" role="status">{t("swap.table.loading")}</p>
              ) : rules.length === 0 ? (
                <p className="rounded-md border border-dashed px-3 py-4 text-center text-sm text-muted-foreground">
                  {t("swap.rules.empty")}
                </p>
              ) : (
                <ul className="divide-y rounded-md border">
                  {rules.map((rule) => (
                    <li key={rule.id} className="flex flex-wrap items-center gap-2 px-3 py-2 text-sm">
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-medium">{rule.name ?? t("swap.rules.unnamed")}</span>
                        <span className="block text-xs text-muted-foreground">{/* loose translator */ ruleSentence(rule, t as unknown as Translate)}</span>
                      </span>
                      <Switch
                        checked={rule.enabled}
                        onCheckedChange={() =>
                          void run(
                            () => ruleSetEnabled.mutateAsync({ rule_id: rule.id, enabled: !rule.enabled }),
                            rulesQuery.refetch,
                          )
                        }
                        aria-label={t("swap.rules.toggleNamedAria", { name: rule.name ?? t("swap.rules.unnamed") })}
                      />
                      {/* Deleting asks once more: a rule sets the tax treatment of future links. */}
                      {confirmDelete === rule.id ? (
                        <span className="flex items-center gap-1">
                          <Button
                            size="sm"
                            variant="destructive"
                            className="h-8"
                            onClick={() => {
                              setConfirmDelete(null);
                              void run(() => ruleDelete.mutateAsync({ rule_id: rule.id }), rulesQuery.refetch);
                            }}
                          >
                            {t("swap.rules.deleteConfirm")}
                          </Button>
                          <Button size="sm" variant="ghost" className="h-8" onClick={() => setConfirmDelete(null)}>
                            {t("common:actions.cancel")}
                          </Button>
                        </span>
                      ) : (
                        <Button
                          size="icon"
                          variant="ghost"
                          className="size-8"
                          onClick={() => setConfirmDelete(rule.id)}
                          aria-label={t("swap.rules.deleteNamedAria", { name: rule.name ?? t("swap.rules.unnamed") })}
                        >
                          <Trash2 className="size-3.5" aria-hidden="true" />
                        </Button>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </section>
            {/* Views saved earlier still filter the suggestions; new ones are not made here. */}
            {views.length === 0 ? null : (
            <section className="space-y-2" aria-labelledby="saved-views-title">
              <h3 id="saved-views-title" className="text-sm font-medium">
                {t("swap.savedView.title")}
              </h3>
              <p className="text-xs text-muted-foreground">{t("swap.savedView.explain")}</p>
              {views.length === 0 ? (
                <p className="rounded-md border border-dashed px-3 py-4 text-center text-sm text-muted-foreground">
                  {t("swap.savedView.empty")}
                </p>
              ) : (
                <ul className="divide-y rounded-md border">
                  {views.map((view) => (
                    <li key={view.id} className="flex items-center gap-2 px-3 py-2 text-sm">
                      <Star className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                      <button
                        type="button"
                        className="min-w-0 flex-1 truncate rounded-sm text-left font-medium hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        onClick={() => onOpenView(view)}
                      >
                        {view.name}
                      </button>
                      <Button
                        size="icon"
                        variant="ghost"
                        className="size-8"
                        onClick={() =>
                          void run(() => savedViewDelete.mutateAsync({ view_id: view.id }), () => {
                            void transferViewsQuery.refetch();
                            void swapViewsQuery.refetch();
                          })
                        }
                        aria-label={t("swap.savedView.deleteAria", { name: view.name })}
                      >
                        <Trash2 className="size-3.5" aria-hidden="true" />
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
            )}
            {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
          </div>
        </SheetContent>
      </Sheet>
      <CreateRuleDialog
        open={createRuleOpen}
        onClose={() => setCreateRuleOpen(false)}
        onCreate={async (payload) => {
          await ruleCreate.mutateAsync({ ...payload });
          void rulesQuery.refetch();
          setCreateRuleOpen(false);
        }}
        isCreating={ruleCreate.isPending}
      />
    </>
  );
}

export type { SavedView as TransfersSavedView };
