import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import {
  Context,
  ContextItem,
  Suggestion,
  Suggestions,
} from "@/components/ai-elements";
import { ProviderModelPicker } from "@/components/ai/ProviderModelPicker";
import { cn } from "@/lib/utils";
import {
  AlertTriangle,
  ArrowUp,
  Brain,
  Cloud,
  Cpu,
  FileSpreadsheet,
  RefreshCw,
  ShieldCheck,
  Square,
  type LucideIcon,
} from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import type { AiProviderKind } from "@/lib/aiCapabilities";

interface PromptOption {
  icon: LucideIcon;
  text: string;
  prompt: string;
}

interface Ai02Props {
  className?: string;
  compact?: boolean;
  placeholder?: string;
  prompts?: PromptOption[];
  selection: { provider: string; model: string } | null;
  onSelectionChange: (next: { provider: string; model: string } | null) => void;
  onSubmit: (prompt: string) => void;
  onAbort?: () => void;
  isStreaming?: boolean;
  thinkingEffort?: "auto" | "low" | "medium" | "high";
  onThinkingEffortChange?: (effort: "auto" | "low" | "medium" | "high") => void;
  showThinkingEffort?: boolean;
  inputPanelElevated?: boolean;
  modelPickerEnabled?: boolean;
}

const DEFAULT_PROMPTS: PromptOption[] = [
  {
    icon: AlertTriangle,
    text: "Review quarantine",
    prompt:
      "List quarantined or under-specified transactions and tell me which price, transfer, or wallet evidence is needed to resolve them.",
  },
  {
    icon: RefreshCw,
    text: "Reprocess journals",
    prompt:
      "Check whether journals need reprocessing after recent imports, pricing changes, metadata edits, or transfer pairing.",
  },
  {
    icon: FileSpreadsheet,
    text: "Prepare tax export",
    prompt:
      "Prepare the Austrian tax export checklist and call out missing journal, pricing, or report prerequisites.",
  },
];

const TEXTAREA_MAX_HEIGHT_PX = 176;

export default function Ai02({
  className,
  compact = false,
  placeholder = "Ask anything",
  prompts = DEFAULT_PROMPTS,
  selection,
  onSelectionChange,
  onSubmit,
  onAbort,
  isStreaming = false,
  thinkingEffort = "auto",
  onThinkingEffortChange,
  showThinkingEffort = false,
  inputPanelElevated = true,
  modelPickerEnabled = true,
}: Ai02Props) {
  const [inputValue, setInputValue] = useState("");
  const [activeProviderKind, setActiveProviderKind] =
    useState<AiProviderKind | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

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
  const showSuggestions = !trimmedInput && !isStreaming && prompts.length > 0;
  const ModelIcon =
    activeProviderKind === "remote"
      ? Cloud
      : activeProviderKind === "tee"
        ? ShieldCheck
        : Cpu;

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
          inputPanelElevated
            ? "shadow-[0_10px_35px_rgba(15,23,42,0.14)]"
            : "shadow-none",
          compact
            ? "min-h-[52px] rounded-[18px] group-focus-within/assistant:min-h-[72px] group-focus-within/assistant:rounded-2xl"
            : "min-h-[72px] rounded-2xl",
        )}
      >
        <div className="relative min-h-0 flex-1">
          <Textarea
            ref={inputRef}
            rows={1}
            value={inputValue}
            onChange={(event) => setInputValue(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            className={cn(
              "max-h-44 min-h-0 w-full resize-none whitespace-pre-wrap break-words border-0 bg-transparent! text-[17px] leading-6 text-foreground shadow-none outline-none transition-[padding,color] duration-200 ease-in-out placeholder:text-muted-foreground/80 focus-visible:ring-0 focus-visible:ring-offset-0",
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
                  className="size-9 rounded-full bg-destructive transition-colors duration-100 ease-out cursor-pointer hover:bg-destructive/90!"
                  onClick={onAbort}
                  aria-label="Stop generating"
                >
                  <Square className="h-3.5 w-3.5 text-destructive-foreground" />
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className={cn(
                    "size-9 rounded-full bg-foreground text-background transition-colors duration-100 ease-out cursor-pointer hover:bg-foreground/90 disabled:bg-muted disabled:text-muted-foreground",
                    canSend && "bg-foreground hover:bg-foreground/90!",
                  )}
                  disabled={!canSubmit}
                  onClick={handleSubmit}
                  aria-label={canQueue ? "Queue message" : "Send message"}
                  title={canQueue ? "Queue message" : "Send message"}
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
          <Context className="min-w-0 flex-1">
            <ContextItem
              icon={<ModelIcon className="h-4 w-4 text-muted-foreground" />}
              label="Model context"
              className="max-w-full"
            >
              <ProviderModelPicker
                value={selection}
                onChange={onSelectionChange}
                enabled={modelPickerEnabled}
                onActiveProviderKindChange={setActiveProviderKind}
              />
            </ContextItem>

            {showThinkingEffort ? (
              <ContextItem
                icon={<Brain className="h-3.5 w-3.5" aria-hidden="true" />}
                label="Thinking"
                className="shrink-0"
              >
                <Select
                  value={thinkingEffort}
                  onValueChange={(value) =>
                    onThinkingEffortChange?.(
                      value as "auto" | "low" | "medium" | "high",
                    )
                  }
                  disabled={isStreaming || !onThinkingEffortChange}
                >
                  <SelectTrigger className="h-auto! min-h-0 w-14 border-none bg-transparent! p-0 text-xs leading-none text-muted-foreground shadow-none hover:text-foreground focus:ring-0 focus-visible:border-transparent focus-visible:ring-0 [&_svg]:size-3">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent align="end">
                    <SelectItem value="auto">Auto</SelectItem>
                    <SelectItem value="low">Low</SelectItem>
                    <SelectItem value="medium">Medium</SelectItem>
                    <SelectItem value="high">High</SelectItem>
                  </SelectContent>
                </Select>
              </ContextItem>
            ) : null}

          </Context>

          <div className="ml-auto flex items-center gap-2">
            {isStreaming && onAbort ? (
              <Button
                variant="ghost"
                size="icon-sm"
                className="rounded-full bg-destructive transition-colors duration-100 ease-out cursor-pointer hover:bg-destructive/90!"
                onClick={onAbort}
                aria-label="Stop generating"
              >
                <Square className="h-3.5 w-3.5 text-destructive-foreground" />
              </Button>
            ) : null}
            {!isStreaming || trimmedInput ? (
              <Button
                variant="ghost"
                size="icon-sm"
                className={cn(
                  "rounded-full bg-foreground text-background transition-colors duration-100 ease-out cursor-pointer hover:bg-foreground/90 disabled:bg-muted disabled:text-muted-foreground",
                  (canSend || canQueue) && "bg-foreground hover:bg-foreground/90!",
                )}
                disabled={!canSubmit}
                onClick={handleSubmit}
                aria-label={canQueue ? "Queue message" : "Send message"}
                title={canQueue ? "Queue message" : "Send message"}
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
          {prompts.map((button) => {
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
