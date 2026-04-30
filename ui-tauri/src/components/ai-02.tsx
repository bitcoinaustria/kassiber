import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
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
  Cpu,
  FileSpreadsheet,
  RefreshCw,
  Square,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { useRef, useState } from "react";

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
  toolsEnabled?: boolean;
  onToolsEnabledChange?: (enabled: boolean) => void;
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

export default function Ai02({
  className,
  compact = false,
  placeholder = "Ask Kassiber",
  prompts = DEFAULT_PROMPTS,
  selection,
  onSelectionChange,
  onSubmit,
  onAbort,
  isStreaming = false,
  toolsEnabled = true,
  onToolsEnabledChange,
  inputPanelElevated = true,
  modelPickerEnabled = true,
}: Ai02Props) {
  const [inputValue, setInputValue] = useState("");
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const handlePromptClick = (prompt: string) => {
    if (inputRef.current) {
      inputRef.current.value = prompt;
      setInputValue(prompt);
      inputRef.current.focus();
    }
  };

  const trimmedInput = inputValue.trim();
  const canSend = Boolean(trimmedInput) && Boolean(selection?.model) && !isStreaming;
  const showSuggestions = !trimmedInput && !isStreaming && prompts.length > 0;

  const handleSubmit = () => {
    if (!canSend) return;
    onSubmit(trimmedInput);
    setInputValue("");
    if (inputRef.current) {
      inputRef.current.value = "";
    }
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
        "group/assistant mx-auto flex w-full max-w-3xl flex-col rounded-[28px] border border-zinc-300/90 bg-zinc-200/78 p-2 shadow-[0_24px_90px_rgba(15,23,42,0.30),0_3px_18px_rgba(15,23,42,0.14)] ring-1 ring-white/90 backdrop-blur-xl transition-all duration-200 ease-out dark:border-white/10 dark:bg-zinc-900/55 dark:ring-white/10",
        compact
          ? "gap-0 rounded-3xl p-1.5 hover:gap-3 hover:rounded-[28px] hover:p-2 focus-within:gap-3 focus-within:rounded-[28px] focus-within:p-2"
          : "gap-3",
        className,
      )}
    >
      <div
        className={cn(
          "flex cursor-text flex-col rounded-2xl border border-border/70 bg-background/90 backdrop-blur-md transition-all duration-200 ease-out dark:bg-background/60",
          inputPanelElevated
            ? "shadow-[0_10px_35px_rgba(15,23,42,0.14)]"
            : "shadow-none",
          compact
            ? "min-h-[62px] group-hover/assistant:min-h-[120px] group-focus-within/assistant:min-h-[120px]"
            : "min-h-[120px]",
        )}
      >
        <div className="relative max-h-[258px] flex-1 overflow-y-auto">
          <Textarea
            ref={inputRef}
            value={inputValue}
            onChange={(event) => setInputValue(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            className={cn(
              "w-full resize-none whitespace-pre-wrap break-words border-0 bg-transparent! text-[16px] text-foreground shadow-none outline-none transition-all duration-200 ease-in-out focus-visible:ring-0 focus-visible:ring-offset-0",
              compact
                ? "min-h-0 px-3 pt-2 pb-0 group-hover/assistant:min-h-[48.4px] group-hover/assistant:p-3 group-focus-within/assistant:min-h-[48.4px] group-focus-within/assistant:p-3"
                : "min-h-[48.4px] p-3",
            )}
          />
        </div>

        <div
          className={cn(
            "flex items-center gap-2 p-2 transition-all duration-200 ease-out",
            compact ? "min-h-[32px] pt-0 pb-1" : "min-h-[40px] pb-1",
          )}
        >
          <Context className="min-w-0 flex-1">
            <ContextItem
              icon={<Cpu className="h-4 w-4 text-muted-foreground" />}
              label="Model context"
              className="max-w-full"
            >
              <ProviderModelPicker
                value={selection}
                onChange={onSelectionChange}
                enabled={modelPickerEnabled}
              />
            </ContextItem>

            <ContextItem
              icon={<Wrench className="h-3.5 w-3.5" aria-hidden="true" />}
              label="Tool context"
              className="shrink-0"
            >
              <label className="flex items-center gap-1.5">
                <span>Tools</span>
                <Switch
                  checked={toolsEnabled}
                  onCheckedChange={onToolsEnabledChange}
                  aria-label="Enable assistant tools"
                  disabled={isStreaming || !onToolsEnabledChange}
                />
              </label>
            </ContextItem>
          </Context>

          <div className="ml-auto flex items-center gap-3">
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
            ) : (
              <Button
                variant="ghost"
                size="icon-sm"
                className={cn(
                  "rounded-full transition-colors duration-100 ease-out cursor-pointer bg-primary",
                  canSend && "bg-primary hover:bg-primary/90!",
                )}
                disabled={!canSend}
                onClick={handleSubmit}
                aria-label="Send message"
              >
                <ArrowUp className="h-4 w-4 text-primary-foreground" />
              </Button>
            )}
          </div>
        </div>
      </div>

      {showSuggestions ? (
        <Suggestions
          className={cn(
            "overflow-hidden transition-all duration-200 ease-out",
            compact
              ? "max-h-0 translate-y-1 opacity-0 group-hover/assistant:max-h-16 group-hover/assistant:translate-y-0 group-hover/assistant:opacity-100 group-focus-within/assistant:max-h-16 group-focus-within/assistant:translate-y-0 group-focus-within/assistant:opacity-100"
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
