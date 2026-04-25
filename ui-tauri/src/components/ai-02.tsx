"use client";

import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import {
  AlertTriangle,
  ArrowUp,
  Cpu,
  FileSpreadsheet,
  RefreshCw,
  type LucideIcon,
} from "lucide-react";
import { useRef, useState } from "react";

interface PromptOption {
  icon: LucideIcon;
  text: string;
  prompt: string;
}

interface ModelOption {
  value: string;
  name: string;
  description: string;
}

interface Ai02Props {
  className?: string;
  compact?: boolean;
  placeholder?: string;
  prompts?: PromptOption[];
  models?: ModelOption[];
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

const DEFAULT_MODELS: ModelOption[] = [
  {
    value: "llama-3.3",
    name: "Llama 3.3",
    description: "Local general review",
  },
  {
    value: "qwen3.6",
    name: "Qwen3.6",
    description: "Local reasoning and coding",
  },
  {
    value: "gemma4",
    name: "Gemma 4",
    description: "Local fast summaries",
  },
  {
    value: "mistral-small-3.1",
    name: "Mistral Small 3.1",
    description: "Local structured checks",
  },
];

export default function Ai02({
  className,
  compact = false,
  placeholder = "Ask Kassiber",
  prompts = DEFAULT_PROMPTS,
  models = DEFAULT_MODELS,
}: Ai02Props) {
  const [inputValue, setInputValue] = useState("");
  const [selectedModel, setSelectedModel] = useState(models[0]);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const handlePromptClick = (prompt: string) => {
    if (inputRef.current) {
      inputRef.current.value = prompt;
      setInputValue(prompt);
      inputRef.current.focus();
    }
  };

  const handleModelChange = (value: string) => {
    const model = models.find((m) => m.value === value);
    if (model) {
      setSelectedModel(model);
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
          "flex cursor-text flex-col rounded-2xl border border-border/70 bg-background/90 shadow-[0_10px_35px_rgba(15,23,42,0.14)] backdrop-blur-md transition-all duration-200 ease-out dark:bg-background/60",
          compact
            ? "min-h-[62px] group-hover/assistant:min-h-[120px] group-focus-within/assistant:min-h-[120px]"
            : "min-h-[120px]",
        )}
      >
        <div className="relative max-h-[258px] flex-1 overflow-y-auto">
          <Textarea
            ref={inputRef}
            value={inputValue}
            onChange={(e) => setInputValue(e.target.value)}
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
          <div className="flex aspect-1 items-center gap-1 rounded-full bg-muted p-1.5 text-xs">
            <Cpu className="h-4 w-4 text-muted-foreground" />
          </div>

          <div className="relative flex items-center">
            <Select
              value={selectedModel.value}
              onValueChange={handleModelChange}
            >
              <SelectTrigger className="w-fit border-none bg-transparent! p-0 text-sm text-muted-foreground hover:text-foreground focus:ring-0 shadow-none">
                <SelectValue>
                  <span>{selectedModel.name}</span>
                </SelectValue>
              </SelectTrigger>
              <SelectContent
                position="popper"
                side="top"
                align="start"
                className="min-w-48"
              >
                {models.map((model) => (
                  <SelectItem key={model.value} value={model.value}>
                    <span>{model.name}</span>
                    <span className="text-muted-foreground block text-xs">
                      {model.description}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="ml-auto flex items-center gap-3">
            <Button
              variant="ghost"
              size="icon-sm"
              className={cn(
                "rounded-full transition-colors duration-100 ease-out cursor-pointer bg-primary",
                inputValue && "bg-primary hover:bg-primary/90!",
              )}
              disabled={!inputValue}
              aria-label="Send message"
            >
              <ArrowUp className="h-4 w-4 text-primary-foreground" />
            </Button>
          </div>
        </div>
      </div>

      <div
        className={cn(
          "flex flex-wrap justify-center gap-2 overflow-hidden transition-all duration-200 ease-out",
          compact
            ? "max-h-0 translate-y-1 opacity-0 group-hover/assistant:max-h-16 group-hover/assistant:translate-y-0 group-hover/assistant:opacity-100 group-focus-within/assistant:max-h-16 group-focus-within/assistant:translate-y-0 group-focus-within/assistant:opacity-100"
            : "max-h-16 translate-y-0 opacity-100",
        )}
        aria-hidden={compact}
      >
        {prompts.map((button) => {
          const IconComponent = button.icon;
          return (
            <Button
              key={button.text}
              variant="ghost"
              className="group flex h-auto items-center gap-2 rounded-full border border-border/70 bg-background/85 px-3 py-2 text-sm text-foreground shadow-sm transition-colors duration-200 ease-out hover:bg-background dark:bg-muted/80 dark:hover:bg-muted"
              onClick={() => handlePromptClick(button.prompt)}
            >
              <IconComponent className="h-4 w-4 text-muted-foreground transition-colors group-hover:text-foreground" />
              <span>{button.text}</span>
            </Button>
          );
        })}
      </div>
    </div>
  );
}
