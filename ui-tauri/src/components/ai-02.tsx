import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Textarea } from "@/components/ui/textarea";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Suggestion, Suggestions } from "@/components/ai-elements";
import { ProviderModelPicker } from "@/components/ai/ProviderModelPicker";
import type { AssistantThinkingEffort } from "@/components/ai/assistantSession";
import {
  buildComposerPromptHistoryEntries,
  isCaretOnTextEdge,
  stepComposerPromptHistory,
  type ComposerPromptHistoryMessage,
  type ComposerPromptHistoryPosition,
} from "@/components/ai/composerPromptHistory";
import { isImeKeyEvent } from "@/lib/imeKeyEvent";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import {
  AlertTriangle,
  FileSpreadsheet,
  ListPlus,
  Paperclip,
  Plus,
  RefreshCw,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
  onComposerOverlayOpenChange?: (open: boolean) => void;
  /** Controlled composer text; pair with onValueChange to persist drafts. */
  value?: string;
  onValueChange?: (value: string) => void;
  onSubmit: (prompt: string) => void;
  onAbort?: () => void;
  isStreaming?: boolean;
  thinkingEffort?: AssistantThinkingEffort;
  onThinkingEffortChange?: (effort: AssistantThinkingEffort) => void;
  showThinkingEffort?: boolean;
  modelPickerEnabled?: boolean;
  /** Open the native picker to attach a file. Omit to disable the button. */
  onAttach?: () => void;
  /** Filename of the currently attached file, shown as a removable chip. */
  attachedFilename?: string | null;
  onClearAttachment?: () => void;
  /**
   * The visible conversation, for terminal-style ArrowUp/ArrowDown recall of
   * earlier prompts. Only `user` messages are used; omit to disable recall.
   */
  historyMessages?: ReadonlyArray<ComposerPromptHistoryMessage>;
}

const DEFAULT_PROMPT_KEYS = [
  { icon: AlertTriangle, key: "reviewQuarantine" },
  { icon: RefreshCw, key: "reprocessJournals" },
  { icon: FileSpreadsheet, key: "prepareTaxExport" },
] as const;

const TEXTAREA_MAX_HEIGHT_PX = 176;

// The send and stop marks and ACTION_BUTTON_CLASS are adapted from T3 Code
// (MIT, Copyright (c) 2026 T3 Tools Inc.),
// `apps/web/src/components/chat/ComposerPrimaryActions.tsx`.

/** Send arrow drawn like T3 Code's, a touch lighter than lucide's ArrowUp. */
function SendArrowIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
      <path
        d="M7 11.5V2.5M7 2.5L3 6.5M7 2.5L11 6.5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function StopIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
      <rect x="2" y="2" width="8" height="8" rx="1.5" />
    </svg>
  );
}

// Round action buttons, after T3 Code's ComposerPrimaryActions: a solid pill
// that lifts on hover, sinks on press, and dims (rather than greys) when
// disabled so its shape stays readable.
const ACTION_BUTTON_CLASS =
  "relative isolate flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-full shadow-xs outline-none transition-all duration-150 hover:scale-105 active:scale-[0.94] active:shadow-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-40 disabled:shadow-none [&_svg]:pointer-events-none";

// Keep the textarea focused when a toolbar action is pressed with the pointer;
// in compact mode losing focus would also fold the toolbar away mid-click.
const keepComposerFocus = (event: React.PointerEvent<HTMLElement>) => {
  event.preventDefault();
};

export default function Ai02({
  className,
  composerClassName,
  compact: compactProp = false,
  alwaysShowSuggestions = false,
  placeholder,
  prompts,
  selection,
  onSelectionChange,
  onComposerOverlayOpenChange,
  value,
  onValueChange,
  onSubmit,
  onAbort,
  isStreaming = false,
  thinkingEffort = "auto",
  onThinkingEffortChange,
  showThinkingEffort = false,
  modelPickerEnabled = true,
  onAttach,
  attachedFilename,
  onClearAttachment,
  historyMessages,
}: Ai02Props) {
  const { t } = useTranslation("assistant");
  const [internalValue, setInternalValue] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  // While the model picker or effort menu is open, focus sits in its portal,
  // outside this composer; a compact composer would otherwise fold its
  // toolbar away under the open menu.
  const [overlayOpen, setOverlayOpen] = useState(false);
  const compact = compactProp && !overlayOpen && !pickerOpen;
  const inputValue = value ?? internalValue;
  const setInputValue = (next: string) => {
    if (value === undefined) setInternalValue(next);
    onValueChange?.(next);
  };
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const historyPositionRef = useRef<ComposerPromptHistoryPosition | null>(null);
  const pendingCaretRef = useRef<number | null>(null);

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
      historyPositionRef.current = null;
      setInputValue(prompt);
      inputRef.current.focus();
    }
  };
  const trimmedInput = inputValue.trim();
  const canSubmit = Boolean(trimmedInput) && Boolean(selection?.model);
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
    if (pendingCaretRef.current !== null) {
      const caret = Math.min(pendingCaretRef.current, input.value.length);
      pendingCaretRef.current = null;
      input.setSelectionRange(caret, caret);
    }
  }, [inputValue]);

  // A recall from one conversation must not stay active in the next, where
  // the text-match fallback could adopt that chat's own prompt as a position.
  const historyScope = historyMessages?.[0]?.id ?? null;
  useEffect(() => {
    historyPositionRef.current = null;
  }, [historyScope]);

  const handleOverlayOpenChange = useCallback(
    (open: boolean) => {
      setOverlayOpen(open);
      onComposerOverlayOpenChange?.(open);
    },
    [onComposerOverlayOpenChange],
  );

  const handleSubmit = () => {
    if (!canSubmit) return;
    onSubmit(trimmedInput);
    historyPositionRef.current = null;
    setInputValue("");
  };

  const navigatePromptHistory = (
    direction: "backward" | "forward",
    event: React.KeyboardEvent<HTMLTextAreaElement>,
  ): boolean => {
    if (!historyMessages) return false;
    if (event.shiftKey || event.altKey || event.metaKey || event.ctrlKey) {
      return false;
    }
    // An attached file makes the composer non-empty: recalling an old prompt
    // into it would send that prompt with a new file, which ArrowUp never meant.
    if (attachedFilename) return false;
    // A typed draft with no active recall can never step.
    if (historyPositionRef.current === null && inputValue.length > 0) {
      return false;
    }
    const input = event.currentTarget;
    if (
      !isCaretOnTextEdge(
        input.value,
        input.selectionStart,
        input.selectionEnd,
        direction === "backward" ? "start" : "end",
      )
    ) {
      return false;
    }
    const step = stepComposerPromptHistory({
      direction,
      entries: buildComposerPromptHistoryEntries(historyMessages),
      position: historyPositionRef.current,
      currentPrompt: inputValue,
    });
    if (!step) return false;
    historyPositionRef.current = step.position;
    pendingCaretRef.current = step.prompt.length;
    setInputValue(step.prompt);
    return true;
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Never act on keys that confirm an IME composition (CJK input, dead keys).
    if (isImeKeyEvent(event)) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      handleSubmit();
      return;
    }
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      if (
        navigatePromptHistory(
          event.key === "ArrowUp" ? "backward" : "forward",
          event,
        )
      ) {
        event.preventDefault();
      }
    }
  };

  const handleComposerKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    // Mod+Shift+M toggles the model picker while focus is anywhere in this
    // composer (including the open picker, whose events bubble back here
    // through the React portal). Scoped to the composer so a hidden dock
    // composer never reacts to it.
    if (isImeKeyEvent(event)) return;
    if (
      (event.metaKey || event.ctrlKey) &&
      event.shiftKey &&
      !event.altKey &&
      event.key.toLowerCase() === "m"
    ) {
      if (!modelPickerEnabled) return;
      event.preventDefault();
      // A held shortcut would flip the picker on every key repeat.
      if (event.repeat) return;
      setPickerOpen((open) => !open);
    }
  };

  const sendShortcut = formatShortcut(["enter"]);
  const newLineShortcut = formatShortcut(["shift", "enter"]);
  const sendLabel = canQueue
    ? t("composer.queueMessage")
    : t("composer.sendMessage");

  const renderStopButton = (extraClass?: string) =>
    isStreaming && onAbort ? (
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className={cn(
              ACTION_BUTTON_CLASS,
              "bg-destructive/90 text-white shadow-destructive/25 hover:bg-destructive",
              extraClass,
            )}
            onPointerDown={keepComposerFocus}
            onClick={onAbort}
            aria-label={t("composer.stopGenerating")}
          >
            <StopIcon />
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">{t("composer.stopGenerating")}</TooltipContent>
      </Tooltip>
    ) : null;

  const renderSendButton = (extraClass?: string) => (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* The span keeps the tooltip reachable while the button is disabled. */}
        <span className={cn("inline-flex", extraClass)}>
          <button
            type="button"
            className={cn(
              ACTION_BUTTON_CLASS,
              "bg-foreground text-background shadow-foreground/20 hover:bg-foreground/90",
            )}
            disabled={!canSubmit}
            onPointerDown={keepComposerFocus}
            onClick={handleSubmit}
            aria-label={sendLabel}
          >
            {canQueue ? (
              <ListPlus className="size-4" aria-hidden="true" />
            ) : (
              <SendArrowIcon />
            )}
          </button>
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">
        <span className="flex items-center gap-2">
          <span>{sendLabel}</span>
          <Kbd className="bg-background/15 text-background">{sendShortcut}</Kbd>
          <span className="opacity-70">{t("composer.newLine")}</span>
          <Kbd className="bg-background/15 text-background">{newLineShortcut}</Kbd>
        </span>
      </TooltipContent>
    </Tooltip>
  );

  return (
    <TooltipProvider delayDuration={300}>
      <div
        className={cn(
          // Transparent layout column (ported from T3Code): the visible surface is
          // the inner composer box below; this wrapper only stacks the box and the
          // suggestion chips with a gap.
          "group/assistant mx-auto flex w-full max-w-3xl flex-col transition-all duration-200 ease-out",
          compact ? "gap-0 focus-within:gap-3" : "gap-3",
          className,
        )}
        onKeyDown={handleComposerKeyDown}
      >
        <div
          className={cn(
            "flex cursor-text flex-col rounded-[22px] transition-all duration-200 ease-out",
            // Focus lives on this whole surface (textarea + toolbar), not the
            // Textarea's own ring — otherwise the outline cuts off above the
            // model/send row. Uses `outline` (not `ring`) because the glass
            // surface below already owns `box-shadow`; a ring would override the
            // hairline+drop-shadow on focus. Brand-red for the a11y affordance.
            "focus-within:outline focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring/55",
            // T3Code's understated frosted-glass surface, in every host so the
            // dock composer and the Assistant page composer are the same
            // material. A composer nested in a floating panel adds
            // `.kb-composer-inset` via composerClassName to drop the second drop
            // shadow.
            "kb-composer-glass",
            compact
              ? "min-h-[52px] group-focus-within/assistant:min-h-[72px]"
              : "min-h-[72px]",
            composerClassName,
          )}
          onMouseDown={(event) => {
            // React bubbles events from portals (the model picker popover)
            // through this tree; only real clicks on the composer count.
            if (!event.currentTarget.contains(event.target as Node)) return;
            // Clicks on padding / chrome still focus the field so the whole box
            // feels like one control. Skip real interactive children.
            const target = event.target as HTMLElement | null;
            if (
              target?.closest(
                "button, a, input, textarea, select, [role='button'], [role='combobox'], [role='menuitem'], [role='option']",
              )
            ) {
              return;
            }
            event.preventDefault();
            inputRef.current?.focus();
          }}
        >
          {attachedFilename ? (
            <div className="relative z-10 flex items-center gap-1.5 px-4 pt-3">
              <span className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-full bg-muted px-2.5 py-1 text-xs text-muted-foreground">
                <Paperclip className="h-3 w-3 shrink-0" aria-hidden="true" />
                <span className="truncate">{attachedFilename}</span>
                {onClearAttachment ? (
                  <button
                    type="button"
                    onClick={onClearAttachment}
                    aria-label={t("composer.removeAttachment")}
                    title={t("composer.removeAttachment")}
                    className="shrink-0 rounded-full p-0.5 outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <X className="h-3 w-3" aria-hidden="true" />
                  </button>
                ) : null}
              </span>
            </div>
          ) : null}
          <div className="relative z-10 min-h-0 flex-1">
            <Textarea
              ref={inputRef}
              rows={1}
              value={inputValue}
              onChange={(event) => {
                // Any typed edit ends recall, even one that restores the
                // recalled wording: only history steps keep a position.
                historyPositionRef.current = null;
                setInputValue(event.target.value);
              }}
              onKeyDown={handleKeyDown}
              placeholder={resolvedPlaceholder}
              className={cn(
                "max-h-44 min-h-0 w-full resize-none whitespace-pre-wrap break-words border-0 bg-transparent! text-base leading-relaxed text-foreground shadow-none outline-none transition-[padding,color] duration-200 ease-in-out placeholder:text-muted-foreground/45 focus-visible:border-transparent focus-visible:ring-0 focus-visible:ring-offset-0 focus-visible:shadow-none",
                compact
                  ? "pr-14 pl-4 pt-3.5 pb-0 group-focus-within/assistant:px-4 group-focus-within/assistant:pt-4 group-focus-within/assistant:pb-1"
                  : "px-4 pt-4 pb-1",
              )}
            />
            {compact ? (
              <div className="absolute top-1/2 right-2.5 -translate-y-1/2 group-focus-within/assistant:hidden">
                {isStreaming && onAbort ? renderStopButton() : renderSendButton()}
              </div>
            ) : null}
          </div>

          <div
            className={cn(
              "relative z-10 flex items-center gap-1 px-2.5 pt-0 transition-all duration-200 ease-out",
              compact
                ? "max-h-0 min-h-0 overflow-hidden pb-0 opacity-0 group-focus-within/assistant:max-h-12 group-focus-within/assistant:min-h-[44px] group-focus-within/assistant:pb-2.5 group-focus-within/assistant:opacity-100"
                : "min-h-[44px] pb-2.5",
            )}
          >
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex shrink-0">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    disabled={!onAttach}
                    onClick={onAttach}
                    className="size-8 shrink-0 rounded-full text-muted-foreground hover:text-foreground"
                    aria-label={t("composer.attach")}
                  >
                    <Plus className="h-4 w-4" />
                  </Button>
                </span>
              </TooltipTrigger>
              <TooltipContent side="top">{t("composer.attach")}</TooltipContent>
            </Tooltip>
            {/* T3Code-style provider/model picker plus adjacent reasoning menu. */}
            <div className="flex min-w-0 flex-1 items-center gap-0.5">
              <ProviderModelPicker
                value={selection}
                onChange={onSelectionChange}
                onOverlayOpenChange={handleOverlayOpenChange}
                enabled={modelPickerEnabled}
                open={pickerOpen}
                onOpenChange={setPickerOpen}
                thinkingEffort={thinkingEffort}
                onThinkingEffortChange={
                  isStreaming ? undefined : onThinkingEffortChange
                }
                showThinkingEffort={showThinkingEffort}
              />
            </div>

            <div className="ml-auto flex shrink-0 items-center gap-2">
              {renderStopButton()}
              {!isStreaming || trimmedInput ? renderSendButton() : null}
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
    </TooltipProvider>
  );
}
