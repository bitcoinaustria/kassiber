/**
 * Tight markdown renderer for assistant chat replies.
 *
 * Streaming output is rendered live, so we keep the renderer hardened
 * against partial/incomplete tokens (unclosed code fences, half-written
 * links). `react-markdown` already handles those — it renders unfinished
 * markdown as plain text rather than throwing.
 */

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { cn } from "@/lib/utils";

interface ChatMarkdownProps {
  content: string;
  className?: string;
}

export function ChatMarkdown({ content, className }: ChatMarkdownProps) {
  return (
    <div
      className={cn(
        "prose prose-sm prose-zinc max-w-none dark:prose-invert",
        "prose-p:my-1.5 prose-headings:my-2",
        "prose-pre:my-2 prose-pre:bg-muted prose-pre:p-3 prose-pre:rounded-md prose-pre:text-xs",
        "prose-code:bg-muted prose-code:px-1 prose-code:py-0.5 prose-code:rounded prose-code:text-[0.85em] prose-code:before:content-none prose-code:after:content-none",
        "prose-ul:my-1.5 prose-ol:my-1.5 prose-li:my-0",
        "prose-a:text-primary prose-a:underline-offset-4",
        className,
      )}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // Force external links to open in a new tab; in Tauri this opens
          // the OS browser via the default link handler.
          a: ({ href, children, ...rest }) => (
            <a
              {...rest}
              href={href}
              target="_blank"
              rel="noreferrer noopener"
            >
              {children}
            </a>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
