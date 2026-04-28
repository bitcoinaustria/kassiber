/**
 * Markdown renderer for assistant chat replies.
 *
 * Streaming output is rendered live, so keep the renderer hardened against
 * partial/incomplete tokens. `react-markdown` renders unfinished markdown as
 * plain text rather than throwing, while the component map below gives the
 * transcript table/headline spacing that the stock typography-free setup lacks.
 */

import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { cn } from "@/lib/utils";

interface ChatMarkdownProps {
  content: string;
  className?: string;
}

const components: Components = {
  h1: ({ className, ...props }) => (
    <h1
      className={cn(
        "mt-7 mb-3 scroll-m-20 text-2xl font-semibold leading-tight first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  h2: ({ className, ...props }) => (
    <h2
      className={cn(
        "mt-6 mb-3 scroll-m-20 text-xl font-semibold leading-tight first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  h3: ({ className, ...props }) => (
    <h3
      className={cn(
        "mt-5 mb-2 scroll-m-20 text-base font-semibold leading-snug first:mt-0",
        className,
      )}
      {...props}
    />
  ),
  h4: ({ className, ...props }) => (
    <h4
      className={cn(
        "mt-4 mb-2 scroll-m-20 text-sm font-semibold leading-snug first:mt-0",
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
  pre: ({ className, ...props }) => (
    <pre
      className={cn(
        "my-4 overflow-x-auto rounded-lg border border-border bg-muted/70 p-3 font-mono text-xs leading-relaxed text-foreground first:mt-0 last:mb-0 [&_code]:bg-transparent [&_code]:p-0",
        className,
      )}
      {...props}
    />
  ),
  code: ({ className, ...props }) => (
    <code
      className={cn(
        "rounded bg-muted px-1 py-0.5 font-mono text-[0.9em]",
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
        "border-t border-border px-3 py-2 align-top text-muted-foreground",
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
        "max-w-none text-[15px] leading-7 text-foreground",
        className,
      )}
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {content}
      </ReactMarkdown>
    </div>
  );
}
