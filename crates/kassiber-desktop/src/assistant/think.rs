//! `ThinkParser` (lib/thinkParser.ts): splits a streamed answer into the visible text and the
//! inline `<think>…</think>` reasoning some models (DeepSeek-R1, older Qwen builds) put in their
//! content. A tag split across chunks is held back until the next chunk decides it.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub content: String,
    pub thinking: String,
}

#[derive(Clone, Debug, Default)]
pub struct ThinkParser {
    in_think: bool,
    pending: String,
}

impl ThinkParser {
    pub fn feed(&mut self, chunk: &str) -> Split {
        let mut s = std::mem::take(&mut self.pending) + chunk;
        let mut out = Split::default();
        while !s.is_empty() {
            let tag = if self.in_think { CLOSE } else { OPEN };
            if let Some(idx) = s.find(tag) {
                let before = &s[..idx];
                if self.in_think {
                    out.thinking.push_str(before);
                } else {
                    out.content.push_str(before);
                }
                s = s[idx + tag.len()..].to_owned();
                self.in_think = !self.in_think;
                continue;
            }
            // The longest suffix that could still grow into the tag waits for the next chunk.
            let mut keep = s.len().min(tag.len() - 1);
            while keep > 0
                && !(s.is_char_boundary(s.len() - keep) && tag.starts_with(&s[s.len() - keep..]))
            {
                keep -= 1;
            }
            let (flush, rest) = s.split_at(s.len() - keep);
            if self.in_think {
                out.thinking.push_str(flush);
            } else {
                out.content.push_str(flush);
            }
            self.pending = rest.to_owned();
            s.clear();
        }
        out
    }

    /// What is still held back once the stream ended.
    pub fn flush(&mut self) -> Split {
        let pending = std::mem::take(&mut self.pending);
        if self.in_think {
            Split {
                content: String::new(),
                thinking: pending,
            }
        } else {
            Split {
                content: pending,
                thinking: String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_inline_reasoning_across_chunks() {
        let mut p = ThinkParser::default();
        let a = p.feed("Hi <thi");
        assert_eq!(
            a,
            Split {
                content: "Hi ".into(),
                thinking: String::new()
            }
        );
        let b = p.feed("nk>pondering</th");
        assert_eq!(
            b,
            Split {
                content: String::new(),
                thinking: "pondering".into()
            }
        );
        let c = p.feed("ink> done");
        assert_eq!(
            c,
            Split {
                content: " done".into(),
                thinking: String::new()
            }
        );
        assert_eq!(p.flush(), Split::default());
    }

    #[test]
    fn a_held_prefix_that_never_becomes_a_tag_is_flushed() {
        let mut p = ThinkParser::default();
        assert_eq!(p.feed("a <").content, "a ");
        assert_eq!(p.flush().content, "<");
        let mut q = ThinkParser::default();
        assert_eq!(q.feed("x < y").content, "x < y");
    }

    #[test]
    fn multibyte_text_is_never_cut() {
        let mut p = ThinkParser::default();
        assert_eq!(p.feed("Grüße €").content, "Grüße €");
    }
}
