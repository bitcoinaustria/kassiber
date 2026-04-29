import type { AiChatMessage } from "@/daemon/stream";

type ChatExportResult = "saved" | "download-started" | "cancelled";

interface FileSystemWritableFileStream {
  write(data: string): Promise<void>;
  close(): Promise<void>;
}

interface FileSystemFileHandle {
  createWritable(): Promise<FileSystemWritableFileStream>;
}

interface SaveFilePickerOptions {
  suggestedName?: string;
  types?: Array<{
    description: string;
    accept: Record<string, string[]>;
  }>;
}

type WindowWithSavePicker = Window &
  typeof globalThis & {
    showSaveFilePicker?: (
      options?: SaveFilePickerOptions,
    ) => Promise<FileSystemFileHandle>;
  };

export function chatExportFilename(exportedAt: Date): string {
  return `kassiber-chat-${exportedAt.toISOString().slice(0, 10)}.md`;
}

export function buildChatExportMarkdown(
  messages: AiChatMessage[],
  exportedAt: Date = new Date(),
): string {
  const transcript = messages
    .map((message) => {
      const header = `## ${message.role === "user" ? "You" : "Kassiber"}${
        message.status !== "done" ? ` (${message.status})` : ""
      }`;
      const body =
        message.content ||
        message.errorMessage ||
        (message.status === "cancelled" ? "Stopped by user." : "");
      const tools = message.toolCalls?.length
        ? [
            "",
            "Tools:",
            ...message.toolCalls.map(
              (tool) => `- ${tool.name}: ${tool.status}`,
            ),
          ].join("\n")
        : "";
      return `${header}\n\n${body}${tools}`;
    })
    .join("\n\n---\n\n");
  return `# Kassiber chat export\n\nExported: ${exportedAt.toISOString()}\n\n${transcript}\n`;
}

function triggerAnchorDownload(
  filename: string,
  contents: string,
): ChatExportResult {
  const blob = new Blob([contents], { type: "text/markdown;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
  return "download-started";
}

export async function saveChatExport(
  messages: AiChatMessage[],
): Promise<ChatExportResult> {
  if (messages.length === 0) return "cancelled";
  const exportedAt = new Date();
  const filename = chatExportFilename(exportedAt);
  const contents = buildChatExportMarkdown(messages, exportedAt);
  const picker = (window as WindowWithSavePicker).showSaveFilePicker;

  if (picker) {
    try {
      const handle = await picker({
        suggestedName: filename,
        types: [
          {
            description: "Markdown",
            accept: { "text/markdown": [".md"] },
          },
        ],
      });
      const writable = await handle.createWritable();
      await writable.write(contents);
      await writable.close();
      return "saved";
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return "cancelled";
      }
      throw error;
    }
  }

  return triggerAnchorDownload(filename, contents);
}
