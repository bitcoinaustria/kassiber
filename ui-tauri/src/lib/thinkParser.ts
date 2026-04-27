/**
 * Streaming `<think>...</think>` content splitter.
 *
 * Many open thinking-capable models (Qwen3, DeepSeek-R1, QwQ, …) emit
 * reasoning inline in the assistant content stream wrapped in `<think>`
 * tags. The OpenAI-compatible wire format does not separate reasoning from
 * the visible answer, so the UI has to do the split itself.
 *
 * `ThinkParser` consumes raw content chunks (in arrival order) and emits
 * how many bytes go to the `content` channel vs the `thinking` channel for
 * that chunk. It buffers any byte that *might* be the start of a tag until
 * it knows where it belongs, so a `<think>` boundary can split across
 * chunks without breaking the channel split.
 *
 * Models that never emit thinking tags pass through unchanged and pay
 * essentially zero cost (one `indexOf` per feed).
 */

const OPEN_TAG = "<think>";
const CLOSE_TAG = "</think>";

export interface ThinkSplit {
  /** Visible-answer characters from this chunk. */
  content: string;
  /** Reasoning-pane characters from this chunk. */
  thinking: string;
}

export class ThinkParser {
  private inThink = false;
  private pending = "";

  feed(chunk: string): ThinkSplit {
    // `this.pending` is the "could-be-tag-prefix" tail from the previous
    // feed. Roll it into `s` and clear it now; we'll only restore it via
    // the no-match branch below, after we decide what survives this feed.
    let s = this.pending + chunk;
    this.pending = "";
    let content = "";
    let thinking = "";

    while (s.length > 0) {
      const tag = this.inThink ? CLOSE_TAG : OPEN_TAG;
      const idx = s.indexOf(tag);
      if (idx >= 0) {
        const before = s.slice(0, idx);
        if (this.inThink) thinking += before;
        else content += before;
        s = s.slice(idx + tag.length);
        this.inThink = !this.inThink;
        continue;
      }
      // No complete tag in `s`. Flush everything except a possible tag
      // prefix at the very end (so the next chunk can complete the tag).
      let prefixLen = Math.min(s.length, tag.length - 1);
      while (prefixLen > 0 && !tag.startsWith(s.slice(s.length - prefixLen))) {
        prefixLen -= 1;
      }
      const flushable = prefixLen === 0 ? s : s.slice(0, s.length - prefixLen);
      if (this.inThink) thinking += flushable;
      else content += flushable;
      this.pending = prefixLen === 0 ? "" : s.slice(s.length - prefixLen);
      s = "";
    }
    return { content, thinking };
  }

  /** Flush any pending buffer at end-of-stream into the current channel. */
  flush(): ThinkSplit {
    const out: ThinkSplit = { content: "", thinking: "" };
    if (!this.pending) return out;
    if (this.inThink) out.thinking = this.pending;
    else out.content = this.pending;
    this.pending = "";
    return out;
  }

  /** True iff the parser is currently inside a `<think>` block. */
  get isInsideThink(): boolean {
    return this.inThink;
  }
}

/** One-shot helper for tests / non-streaming inputs. */
export function splitThinkOnce(text: string): ThinkSplit {
  const parser = new ThinkParser();
  const live = parser.feed(text);
  const tail = parser.flush();
  return {
    content: live.content + tail.content,
    thinking: live.thinking + tail.thinking,
  };
}
