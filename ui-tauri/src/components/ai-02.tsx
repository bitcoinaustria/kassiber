import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Suggestion, Suggestions } from "@/components/ai-elements";
import { ProviderModelPicker } from "@/components/ai/ProviderModelPicker";
import { cn } from "@/lib/utils";
import {
  AlertTriangle,
  ArrowUp,
  FileSpreadsheet,
  Plus,
  RefreshCw,
  Square,
  type LucideIcon,
} from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

interface PromptOption {
  icon: LucideIcon;
  text: string;
  prompt: string;
}

interface Ai02Props {
  className?: string;
  /** Extra classes for the inner composer surface (border/fill/shadow). */
  composerClassName?: string;
  compact?: boolean;
  /** Keep the suggestion chips visible even while the composer has text. */
  alwaysShowSuggestions?: boolean;
  placeholder?: string;
  prompts?: PromptOption[];
  selection: { provider: string; model: string } | null;
  onSelectionChange: (next: { provider: string; model: string } | null) => void;
  /** Controlled composer text; pair with onValueChange to persist drafts. */
  value?: string;
  onValueChange?: (value: string) => void;
  onSubmit: (prompt: string) => void;
  onAbort?: () => void;
  isStreaming?: boolean;
  thinkingEffort?: "auto" | "low" | "medium" | "high";
  onThinkingEffortChange?: (effort: "auto" | "low" | "medium" | "high") => void;
  showThinkingEffort?: boolean;
  inputPanelElevated?: boolean;
  modelPickerEnabled?: boolean;
}

const DEFAULT_PROMPT_KEYS = [
  { icon: AlertTriangle, key: "reviewQuarantine" },
  { icon: RefreshCw, key: "reprocessJournals" },
  { icon: FileSpreadsheet, key: "prepareTaxExport" },
] as const;

const TEXTAREA_MAX_HEIGHT_PX = 176;

export default function Ai02({
  className,
  composerClassName,
  compact = false,
  alwaysShowSuggestions = false,
  placeholder,
  prompts,
  selection,
  onSelectionChange,
  value,
  onValueChange,
  onSubmit,
  onAbort,
  isStreaming = false,
  thinkingEffort = "auto",
  onThinkingEffortChange,
  showThinkingEffort = false,
  inputPanelElevated = true,
  modelPickerEnabled = true,
}: Ai02Props) {
  const { t } = useTranslation("assistant");
  const [internalValue, setInternalValue] = useState("");
  const inputValue = value ?? internalValue;
  const setInputValue = (next: string) => {
    if (value === undefined) setInternalValue(next);
    onValueChange?.(next);
  };
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const resolvedPlaceholder = placeholder ?? t("composer.placeholder");
  const defaultPrompts = useMemo<PromptOption[]>(
    () =>
      DEFAULT_PROMPT_KEYS.map(({ icon, key }) => ({
        icon,
        text: t(`prompts.${key}.text`),
        prompt: t(`prompts.${key}.prompt`),
      })),
    [t],
  );
  const resolvedPrompts = prompts ?? defaultPrompts;

  const handlePromptClick = (prompt: string) => {
    if (inputRef.current) {
      inputRef.current.value = prompt;
      setInputValue(prompt);
      inputRef.current.focus();
    }
  };
  const trimmedInput = inputValue.trim();
  const canSubmit = Boolean(trimmedInput) && Boolean(selection?.model);
  const canSend = canSubmit && !isStreaming;
  const canQueue = canSubmit && isStreaming;
  const showSuggestions =
    (alwaysShowSuggestions || !trimmedInput) &&
    !isStreaming &&
    resolvedPrompts.length > 0;

  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.style.height = "auto";
    input.style.height = `${Math.min(input.scrollHeight, TEXTAREA_MAX_HEIGHT_PX)}px`;
    input.style.overflowY =
      input.scrollHeight > TEXTAREA_MAX_HEIGHT_PX ? "auto" : "hidden";
  }, [inputValue]);

  const handleSubmit = () => {
    if (!canSubmit) return;
    onSubmit(trimmedInput);
    setInputValue("");
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      handleSubmit();
    }
  };

  return (
    <div
      className={cn(
        "group/assistant mx-auto flex w-full max-w-3xl flex-col rounded-[28px] border border-zinc-300/90 bg-zinc-200/78 p-2 shadow-[0_24px_90px_rgba(15,23,42,0.30),0_3px_18px_rgba(15,23,42,0.14),inset_0_1px_0_rgba(255,255,255,0.65)] ring-1 ring-white/90 backdrop-blur-xl transition-all duration-200 ease-out dark:border-white/16 dark:bg-zinc-950/86 dark:shadow-[0_24px_80px_rgba(0,0,0,0.68),0_6px_24px_rgba(0,0,0,0.44),inset_0_1px_0_rgba(255,255,255,0.11)] dark:ring-white/12",
        compact
          ? "gap-0 rounded-[24px] p-1.5 focus-within:gap-3 focus-within:rounded-[28px] focus-within:p-2"
          : "gap-3",
        className,
      )}
    >
      <div
        className={cn(
          "flex cursor-text flex-col border border-border/70 bg-background/90 backdrop-blur-md transition-all duration-200 ease-out dark:bg-background/60",
          // Focus lives on this whole muted surface (textarea + toolbar), not
          // the Textarea's own ring — otherwise the blue outline cuts off above
          // the model/send row.
          "focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50",
          inputPanelElevated
            ? "shadow-[0_10px_35px_rgba(15,23,42,0.14)]"
            : "shadow-none",
          compact
            ? "min-h-[52px] rounded-[18px] group-focus-within/assistant:min-h-[72px] group-focus-within/assistant:rounded-2xl"
            : "min-h-[72px] rounded-2xl",
          composerClassName,
        )}
        onMouseDown={(event) => {
          // Clicks on padding / chrome still focus the field so the whole box
          // feels like one control. Skip real interactive children.
          const target = event.target as HTMLElement | null;
          if (
            target?.closest(
              "button, a, input, textarea, select, [role='button'], [role='combobox'], [role='menuitem']",
            )
          ) {
            return;
          }
          event.preventDefault();
          inputRef.current?.focus();
        }}
      >
        <div className="relative min-h-0 flex-1">
          <Textarea
            ref={inputRef}
            rows={1}
            value={inputValue}
            onChange={(event) => setInputValue(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={resolvedPlaceholder}
            className={cn(
              "max-h-44 min-h-0 w-full resize-none whitespace-pre-wrap break-words border-0 bg-transparent! text-[17px] leading-6 text-foreground shadow-none outline-none transition-[padding,color] duration-200 ease-in-out placeholder:text-muted-foreground/80 focus-visible:border-transparent focus-visible:ring-0 focus-visible:ring-offset-0 focus-visible:shadow-none",
              compact
                ? "pr-14 pl-4 pt-3.5 pb-0 group-focus-within/assistant:px-4 group-focus-within/assistant:pt-4 group-focus-within/assistant:pb-1"
                : "px-4 pt-4 pb-1",
            )}
          />
          {compact ? (
            <div className="absolute top-1/2 right-2 -translate-y-1/2 group-focus-within/assistant:hidden">
              {isStreaming && onAbort ? (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="size-9 cursor-pointer rounded-full bg-destructive transition-all duration-100 ease-out hover:bg-destructive/90! active:scale-[0.94]"
                  onClick={onAbort}
                  aria-label={t("composer.stopGenerating")}
                >
                  <Square className="h-3.5 w-3.5 text-destructive-foreground" />
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className={cn(
                    "size-9 cursor-pointer rounded-full bg-foreground text-background transition-all duration-100 ease-out hover:bg-foreground/90 active:scale-[0.94] disabled:bg-muted disabled:text-muted-foreground",
                    canSend && "bg-foreground hover:bg-foreground/90!",
                  )}
                  disabled={!canSubmit}
                  onClick={handleSubmit}
                  aria-label={
                    canQueue
                      ? t("composer.queueMessage")
                      : t("composer.sendMessage")
                  }
                  title={
                    canQueue
                      ? t("composer.queueMessage")
                      : t("composer.sendMessage")
                  }
                >
                  <ArrowUp className="h-4 w-4" />
                </Button>
              )}
            </div>
          ) : null}
        </div>

        <div
          className={cn(
            "flex items-center gap-2 px-2 pt-0 transition-all duration-200 ease-out",
            compact
              ? "max-h-0 min-h-0 overflow-hidden pb-0 opacity-0 group-focus-within/assistant:max-h-11 group-focus-within/assistant:min-h-[42px] group-focus-within/assistant:pb-2 group-focus-within/assistant:opacity-100"
              : "min-h-[42px] pb-2",
          )}
        >
          {/* Attachment entry point. Mock for now — no upload wired yet. */}
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="size-8 shrink-0 rounded-full text-muted-foreground hover:text-foreground"
            aria-label={t("composer.attach")}
            title={t("composer.attach")}
          >
            <Plus className="h-4 w-4" />
          </Button>
          {/* Combined model + reasoning-effort control (one dropdown). */}
          <div className="flex min-w-0 flex-1 items-center">
            <ProviderModelPicker
              value={selection}
              onChange={onSelectionChange}
              enabled={modelPickerEnabled}
              thinkingEffort={thinkingEffort}
              onThinkingEffortChange={
                isStreaming ? undefined : onThinkingEffortChange
              }
              showThinkingEffort={showThinkingEffort}
            />
          </div>

          <div className="ml-auto flex items-center gap-2">
            {isStreaming && onAbort ? (
              <Button
                variant="ghost"
                size="icon-sm"
                className="cursor-pointer rounded-full bg-destructive transition-all duration-100 ease-out hover:bg-destructive/90! active:scale-[0.94]"
                onClick={onAbort}
                aria-label={t("composer.stopGenerating")}
              >
                <Square className="h-3.5 w-3.5 text-destructive-foreground" />
              </Button>
            ) : null}
            {!isStreaming || trimmedInput ? (
              <Button
                variant="ghost"
                size="icon-sm"
                className={cn(
                  "cursor-pointer rounded-full bg-foreground text-background transition-all duration-100 ease-out hover:bg-foreground/90 active:scale-[0.94] disabled:bg-muted disabled:text-muted-foreground",
                  (canSend || canQueue) && "bg-foreground hover:bg-foreground/90!",
                )}
                disabled={!canSubmit}
                onClick={handleSubmit}
                aria-label={
                  canQueue
                    ? t("composer.queueMessage")
                    : t("composer.sendMessage")
                }
                title={
                  canQueue
                    ? t("composer.queueMessage")
                    : t("composer.sendMessage")
                }
              >
                <ArrowUp className="h-4 w-4" />
              </Button>
            ) : null}
          </div>
        </div>
      </div>

      {showSuggestions ? (
        <Suggestions
          className={cn(
            "overflow-hidden transition-all duration-200 ease-out",
            compact
              ? "max-h-0 translate-y-1 opacity-0 group-focus-within/assistant:max-h-16 group-focus-within/assistant:translate-y-0 group-focus-within/assistant:opacity-100"
              : "max-h-16 translate-y-0 opacity-100",
          )}
          aria-hidden={compact}
        >
          {resolvedPrompts.map((button) => {
            const IconComponent = button.icon;
            return (
              <Suggestion
                key={button.text}
                suggestion={button.prompt}
                onClick={handlePromptClick}
              >
                <IconComponent className="h-4 w-4 text-muted-foreground transition-colors group-hover:text-foreground" />
                <span>{button.text}</span>
              </Suggestion>
            );
          })}
        </Suggestions>
      ) : null}
    </div>
  );
}
