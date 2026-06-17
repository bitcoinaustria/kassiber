import { Plus, X } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";

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
import { TabsContent } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

import { DirtyDot, InfoHint } from "./TransactionDetailSheetParts";
import {
  allTransactionStatuses,
  blurClass,
  classificationOptions,
  classificationOptionLabelKeys,
  tagSuggestionLabelKeys,
  transactionStatusLabels,
  type TransactionStatus,
} from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

export function TransactionClassifyTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
  const {
    localDraft,
    dirty,
    dirtyLabel,
    dirtyTags,
    dirtyNote,
    hideSensitive,
    tags,
    updateDraft,
    tagInput,
    setTagInput,
    tagInputRef,
    addTag,
    removeTag,
    availableTagSuggestions,
  } = ctx;
  return (
    <>
                  {/* Classify — label, tags, note, review status. NO tax handling. */}
                  <TabsContent value="classify" className="mt-4">
                    <div className="grid gap-4 lg:grid-cols-2">
                      <div className="grid gap-2">
                        <Label
                          htmlFor="tx-label"
                          className="flex items-center gap-1.5"
                        >
                          {t("classify.label")}
                          <DirtyDot active={dirtyLabel} />
                        </Label>
                        <Select
                          value={localDraft.label}
                          onValueChange={(value) => updateDraft("label", value)}
                        >
                          <SelectTrigger id="tx-label">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {classificationOptions.map((label) => (
                              <SelectItem key={label} value={label}>
                                {classificationOptionLabelKeys[label]
                                  ? t(classificationOptionLabelKeys[label])
                                  : label}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-2">
                        <Label
                          htmlFor="tx-status"
                          className="flex items-center gap-1.5 text-muted-foreground"
                        >
                          {t("classify.reviewStatus")}
                          <DirtyDot active={dirty.reviewStatus} />
                          <InfoHint label={t("classify.reviewStatus")}>
                            {t("classify.reviewStatusHint")}
                          </InfoHint>
                        </Label>
                        <Select
                          value={localDraft.reviewStatus}
                          onValueChange={(value) =>
                            updateDraft(
                              "reviewStatus",
                              value as TransactionStatus,
                            )
                          }
                        >
                          <SelectTrigger id="tx-status">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {allTransactionStatuses.map((status) => (
                              <SelectItem key={status} value={status}>
                                {t(transactionStatusLabels[status])}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-2 lg:col-span-2">
                        <Label
                          htmlFor="tx-tag-input"
                          className="flex items-center gap-1.5"
                        >
                          {t("classify.tags")}
                          <DirtyDot active={dirtyTags} />
                          <span className="text-xs font-normal text-muted-foreground">
                            <Trans
                              i18nKey="classify.tagsFocusHint"
                              ns="transactions"
                              components={[
                                <kbd className="rounded border bg-muted px-1" />,
                              ]}
                            />
                          </span>
                        </Label>
                        <div className="rounded-md border bg-background p-2">
                          <div className="mb-2 flex min-h-8 flex-wrap gap-1.5">
                            {tags.length ? (
                              tags.map((tag) => (
                                <button
                                  key={tag}
                                  type="button"
                                  className={cn(
                                    "inline-flex items-center gap-1 rounded-md bg-secondary px-2 py-1 text-xs font-medium text-secondary-foreground",
                                    blurClass(hideSensitive),
                                  )}
                                  onClick={() => removeTag(tag)}
                                  aria-label={t("classify.removeTagAria", { tag })}
                                >
                                  {tag}
                                  <X className="size-3" aria-hidden="true" />
                                </button>
                              ))
                            ) : (
                              <span className="px-1 py-1 text-sm text-muted-foreground">
                                {t("classify.noTagsYet")}
                              </span>
                            )}
                          </div>
                          <div className="flex gap-2">
                            <Input
                              id="tx-tag-input"
                              ref={tagInputRef}
                              value={tagInput}
                              className={blurClass(hideSensitive)}
                              onChange={(event) =>
                                setTagInput(event.target.value)
                              }
                              onKeyDown={(event) => {
                                if (
                                  event.key === "Enter" ||
                                  event.key === ","
                                ) {
                                  event.preventDefault();
                                  addTag(tagInput);
                                }
                              }}
                              placeholder={t("classify.addTagPlaceholder")}
                            />
                            <Button
                              type="button"
                              variant="outline"
                              size="icon"
                              aria-label={t("classify.addTagAria")}
                              onClick={() => addTag(tagInput)}
                            >
                              <Plus className="size-4" aria-hidden="true" />
                            </Button>
                          </div>
                        </div>
                        <div className="flex flex-wrap gap-1.5">
                          {availableTagSuggestions.slice(0, 7).map((tag) => (
                            <button
                              key={tag}
                              type="button"
                              className="rounded-md border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                              onClick={() => addTag(tag)}
                            >
                              {t("classify.suggestionPrefix", {
                                tag: tagSuggestionLabelKeys[tag]
                                  ? t(tagSuggestionLabelKeys[tag])
                                  : tag,
                              })}
                            </button>
                          ))}
                        </div>
                      </div>
                      <div className="grid gap-2 lg:col-span-2">
                        <Label
                          htmlFor="tx-note"
                          className="flex items-center gap-1.5"
                        >
                          {t("classify.note")}
                          <DirtyDot active={dirtyNote} />
                        </Label>
                        <Textarea
                          id="tx-note"
                          value={localDraft.note}
                          onChange={(event) =>
                            updateDraft("note", event.target.value)
                          }
                          className={cn(
                            "min-h-28 resize-none",
                            blurClass(hideSensitive),
                          )}
                          placeholder={t("classify.notePlaceholder")}
                        />
                      </div>
                    </div>
                  </TabsContent>


    </>
  );
}
