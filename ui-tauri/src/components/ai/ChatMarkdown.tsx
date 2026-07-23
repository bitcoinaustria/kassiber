/**
 * Markdown renderer for assistant chat replies.
 *
 * Streaming output is rendered live, so keep the renderer hardened against
 * partial/incomplete tokens. `react-markdown` renders unfinished markdown as
 * plain text rather than throwing, while the component map below gives the
 * transcript table/headline spacing that the stock typography-free setup lacks.
 */

import * as React from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, Copy } from "lucide-react";
import { useTranslation } from "react-i18next";

import { copyTextWithPolicy } from "@/lib/clipboard";
import { cn } from "@/lib/utils";

interface ChatMarkdownProps {
  content: string;
  className?: string;
}

function nodeToString(node: React.ReactNode): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(nodeToString).join("");
  if (React.isValidElement(node)) {
    return nodeToString(
      (node.props as { children?: React.ReactNode }).children,
    );
  }
  return "";
}

/**
 * Fenced code block with T3Code-style chrome: a header carrying the language
 * label and a copy affordance, over the code surface. Syntax highlighting
 * (Shiki in T3Code) is intentionally omitted — Kassiber's assistant rarely
 * emits source code, so the heavy highlighter isn't worth the weight.
 */
function ChatCodeBlock({
  children,
  className,
}: {
  children?: React.ReactNode;
  className?: string;
}) {
  const { t } = useTranslation("assistant");
  const [copied, setCopied] = React.useState(false);
  const timerRef = React.useRef<number | null>(null);

  const codeChild = React.Children.toArray(children).find(
    React.isValidElement,
  ) as
    | React.ReactElement<{ className?: string; children?: React.ReactNode }>
    | undefined;
  const language =
    /language-([\w-]+)/.exec(codeChild?.props.className ?? "")?.[1] ?? null;
  const raw = nodeToString(codeChild?.props.children ?? children).replace(
    /\n$/,
    "",
  );

  React.useEffect(
    () => () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    },
    [],
  );

  const handleCopy = () => {
    void copyTextWithPolicy(raw);
    setCopied(true);
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      setCopied(false);
      timerRef.current = null;
    }, 1500);
  };

  const copyLabel = copied ? t("message.copied") : t("message.copy");

  return (
    <div className="my-4 overflow-hidden rounded-xl border border-border bg-muted/70 first:mt-0 last:mb-0">
      <div className="flex items-center justify-between gap-2 border-b border-border/60 bg-muted/50 py-1 pr-1.5 pl-3">
        <span className="font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
          {language ?? t("message.code")}
        </span>
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[10px] font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={copyLabel}
          title={copyLabel}
        >
          {copied ? (
            <Check className="h-3 w-3" aria-hidden="true" />
          ) : (
            <Copy className="h-3 w-3" aria-hidden="true" />
          )}
          <span>{copyLabel}</span>
        </button>
      </div>
      <pre
        className={cn(
          "overflow-x-auto p-3 font-mono text-xs leading-relaxed text-foreground [&_code]:border-0 [&_code]:bg-transparent [&_code]:p-0",
          className,
        )}
      >
        {children}
      </pre>
    </div>
  );
}

const components: Components = {
  h1: ({ className, ...props }) => (
    <h1
      className={cn(
        "mt-6 mb-3 scroll-m-20 text-lg font-semibold leading-snug first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  h2: ({ className, ...props }) => (
    <h2
      className={cn(
        "mt-5 mb-2 scroll-m-20 text-base font-semibold leading-snug first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  h3: ({ className, ...props }) => (
    <h3
      className={cn(
        "mt-4 mb-2 scroll-m-20 text-sm font-semibold leading-snug first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  h4: ({ className, ...props }) => (
    <h4
      className={cn(
        "mt-4 mb-2 scroll-m-20 text-sm font-semibold leading-snug text-muted-foreground first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  p: ({ className, ...props }) => (
    <p className={cn("my-3 first:mt-0 last:mb-0", className)} {...props} />
  ),
  ul: ({ className, ...props }) => (
    <ul
      className={cn(
        "my-3 ml-5 list-disc space-y-1.5 first:mt-0 last:mb-0",
        className,
      )}
      {...props}
    />
  ),
  ol: ({ className, ...props }) => (
    <ol
      className={cn(
        "my-3 ml-5 list-decimal space-y-1.5 first:mt-0 last:mb-0",
        className,
      )}
      {...props}
    />
  ),
  li: ({ className, ...props }) => (
    <li className={cn("pl-1", className)} {...props} />
  ),
  blockquote: ({ className, ...props }) => (
    <blockquote
      className={cn(
        "my-4 border-l-2 border-border pl-4 text-muted-foreground first:mt-0 last:mb-0",
        className,
      )}
      {...props}
    />
  ),
  hr: ({ className, ...props }) => (
    <hr className={cn("my-6 border-border", className)} {...props} />
  ),
  pre: ({ className, children }) => (
    <ChatCodeBlock className={className}>{children}</ChatCodeBlock>
  ),
  code: ({ className, ...props }) => (
    <code
      className={cn(
        "rounded-md border border-border bg-muted px-1 py-0.5 font-mono text-[0.85em]",
        className,
      )}
      {...props}
    />
  ),
  table: ({ className, ...props }) => (
    <div className="my-4 overflow-x-auto rounded-lg border border-border first:mt-0 last:mb-0">
      <table
        className={cn(
          "min-w-full border-collapse text-sm leading-6",
          className,
        )}
        {...props}
      />
    </div>
  ),
  thead: ({ className, ...props }) => (
    <thead className={cn("bg-muted/70", className)} {...props} />
  ),
  th: ({ className, ...props }) => (
    <th
      className={cn(
        "whitespace-nowrap px-3 py-2 text-left font-medium text-foreground",
        className,
      )}
      {...props}
    />
  ),
  td: ({ className, ...props }) => (
    <td
      className={cn(
        "border-t border-border px-3 py-2 align-top text-foreground tabular-nums",
        className,
      )}
      {...props}
    />
  ),
  a: ({ href, children, ...rest }) => (
    <a
      {...rest}
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      className="font-medium text-primary underline underline-offset-4"
    >
      {children}
    </a>
  ),
};

export function ChatMarkdown({ content, className }: ChatMarkdownProps) {
  return (
    <div
      className={cn(
        "max-w-none text-sm leading-relaxed text-foreground/85",
        className,
      )}
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {content}
      </ReactMarkdown>
    </div>
  );
}
