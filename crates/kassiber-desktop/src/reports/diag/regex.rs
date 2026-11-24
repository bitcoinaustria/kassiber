//! A small backtracking regular-expression engine with JavaScript semantics, for the Logs
//! screen's regex search and the log redaction patterns (lib/appLogs.ts). This crate has no
//! `regex` dependency, and the redaction patterns need look-behind, which `regex` lacks anyway.
//!
//! Supported: literals and escapes (`\d \w \s \D \W \S \b \B \n \t \r \f \v \uXXXX \xXX`),
//! `.`, classes with ranges and negation, groups (`(…)`, `(?:…)`), alternation, greedy and
//! lazy quantifiers (`* + ? {m} {m,} {m,n}`), anchors, look-ahead and look-behind, and the
//! case-insensitive flag. Matching runs on chars; offsets are char indices.

#[derive(Clone, Debug)]
enum Item {
    Ch(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Clone, Debug)]
enum Node {
    Ch(char),
    Any,
    Class {
        items: Vec<Item>,
        neg: bool,
    },
    Start,
    End,
    WordB(bool),
    Group(Vec<Vec<Node>>),
    Repeat {
        alts: Vec<Vec<Node>>,
        min: usize,
        max: usize,
        greedy: bool,
    },
    Look {
        ahead: bool,
        neg: bool,
        alts: Vec<Vec<Node>>,
    },
}

#[derive(Clone, Debug)]
pub struct Regex {
    alts: Vec<Vec<Node>>,
    icase: bool,
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn alts(&mut self) -> Result<Vec<Vec<Node>>, String> {
        let mut alts = vec![self.seq()?];
        while self.eat('|') {
            alts.push(self.seq()?);
        }
        Ok(alts)
    }
    fn seq(&mut self) -> Result<Vec<Node>, String> {
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let atom = self.quant(atom)?;
            out.push(atom);
        }
        Ok(out)
    }
    fn number(&mut self) -> Option<usize> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if start == self.i {
            None
        } else {
            self.s[start..self.i]
                .iter()
                .collect::<String>()
                .parse()
                .ok()
        }
    }
    fn quant(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.i += 1;
                (0, usize::MAX)
            }
            Some('+') => {
                self.i += 1;
                (1, usize::MAX)
            }
            Some('?') => {
                self.i += 1;
                (0, 1)
            }
            Some('{') => {
                let save = self.i;
                self.i += 1;
                let Some(min) = self.number() else {
                    self.i = save;
                    return Ok(atom);
                };
                let max = if self.eat(',') {
                    self.number().unwrap_or(usize::MAX)
                } else {
                    min
                };
                if !self.eat('}') {
                    self.i = save;
                    return Ok(atom);
                }
                if max < min {
                    return Err("numbers out of order in {} quantifier".into());
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        if matches!(atom, Node::Start | Node::End | Node::WordB(_)) {
            return Err("nothing to repeat".into());
        }
        let greedy = !self.eat('?');
        let alts = match atom {
            Node::Group(a) => a,
            other => vec![vec![other]],
        };
        Ok(Node::Repeat {
            alts,
            min,
            max,
            greedy,
        })
    }
    fn escape_char(&mut self) -> Result<Item, String> {
        let c = self.peek().ok_or("\\ at end of pattern")?;
        self.i += 1;
        Ok(match c {
            'd' => Item::Digit(true),
            'D' => Item::Digit(false),
            'w' => Item::Word(true),
            'W' => Item::Word(false),
            's' => Item::Space(true),
            'S' => Item::Space(false),
            'n' => Item::Ch('\n'),
            't' => Item::Ch('\t'),
            'r' => Item::Ch('\r'),
            'f' => Item::Ch('\u{c}'),
            'v' => Item::Ch('\u{b}'),
            '0' => Item::Ch('\0'),
            'u' | 'x' => {
                let n = if c == 'u' { 4 } else { 2 };
                let hex: String = self
                    .s
                    .get(self.i..self.i + n)
                    .map(|h| h.iter().collect())
                    .unwrap_or_default();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) if hex.len() == n => {
                        self.i += n;
                        Item::Ch(ch)
                    }
                    _ => Item::Ch(c),
                }
            }
            other => Item::Ch(other),
        })
    }
    fn atom(&mut self) -> Result<Node, String> {
        let c = self.peek().ok_or("unexpected end")?;
        self.i += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '*' | '+' | '?' => return Err("nothing to repeat".into()),
            '(' => {
                let node = if self.eat('?') {
                    if self.eat(':') {
                        Node::Group(self.alts()?)
                    } else if self.eat('=') {
                        Node::Look {
                            ahead: true,
                            neg: false,
                            alts: self.alts()?,
                        }
                    } else if self.eat('!') {
                        Node::Look {
                            ahead: true,
                            neg: true,
                            alts: self.alts()?,
                        }
                    } else if self.eat('<') {
                        if self.eat('=') {
                            Node::Look {
                                ahead: false,
                                neg: false,
                                alts: self.alts()?,
                            }
                        } else if self.eat('!') {
                            Node::Look {
                                ahead: false,
                                neg: true,
                                alts: self.alts()?,
                            }
                        } else {
                            return Err("named groups are not supported".into());
                        }
                    } else {
                        return Err("invalid group".into());
                    }
                } else {
                    Node::Group(self.alts()?)
                };
                if !self.eat(')') {
                    return Err("unterminated group".into());
                }
                node
            }
            ')' => return Err("unmatched )".into()),
            '[' => {
                let neg = self.eat('^');
                let mut items = Vec::new();
                let mut first = true;
                loop {
                    let c = self.peek().ok_or("unterminated character class")?;
                    if c == ']' && !first {
                        self.i += 1;
                        break;
                    }
                    if c == ']' {
                        // `[]` matches nothing in JS; `[^]` anything.
                        self.i += 1;
                        break;
                    }
                    first = false;
                    self.i += 1;
                    let lo = if c == '\\' {
                        self.escape_char()?
                    } else {
                        Item::Ch(c)
                    };
                    if let Item::Ch(lo_c) = lo
                        && self.peek() == Some('-')
                        && self.s.get(self.i + 1).is_some_and(|&n| n != ']')
                    {
                        self.i += 1;
                        let hc = self.peek().ok_or("bad range")?;
                        self.i += 1;
                        let hi = if hc == '\\' {
                            self.escape_char()?
                        } else {
                            Item::Ch(hc)
                        };
                        match hi {
                            Item::Ch(hi_c) if hi_c >= lo_c => items.push(Item::Range(lo_c, hi_c)),
                            Item::Ch(_) => {
                                return Err("range out of order in character class".into());
                            }
                            other => {
                                items.push(Item::Ch(lo_c));
                                items.push(Item::Ch('-'));
                                items.push(other);
                            }
                        }
                        continue;
                    }
                    items.push(lo);
                }
                Node::Class { items, neg }
            }
            '\\' => match self.peek() {
                Some('b') => {
                    self.i += 1;
                    Node::WordB(true)
                }
                Some('B') => {
                    self.i += 1;
                    Node::WordB(false)
                }
                _ => match self.escape_char()? {
                    Item::Ch(c) => Node::Ch(c),
                    other => Node::Class {
                        items: vec![other],
                        neg: false,
                    },
                },
            },
            c => Node::Ch(c),
        })
    }
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

impl Regex {
    pub fn new(pattern: &str, icase: bool) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0 };
        let alts = p.alts()?;
        if p.i != chars.len() {
            return Err("unmatched )".into());
        }
        Ok(Regex { alts, icase })
    }

    fn item_matches(&self, it: &Item, c: char) -> bool {
        match *it {
            Item::Ch(x) => x == c || (self.icase && fold(x) == fold(c)),
            Item::Range(lo, hi) => {
                (lo..=hi).contains(&c)
                    || (self.icase
                        && ((lo..=hi).contains(&fold(c)) || (lo..=hi).contains(&upper(c))))
            }
            Item::Digit(p) => c.is_ascii_digit() == p,
            Item::Word(p) => is_word(c) == p,
            Item::Space(p) => is_space(c) == p,
        }
    }

    fn alts(
        &self,
        s: &[char],
        alts: &[Vec<Node>],
        pos: usize,
        k: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        alts.iter().any(|a| self.seq(s, a, pos, k))
    }

    fn seq(
        &self,
        s: &[char],
        nodes: &[Node],
        pos: usize,
        k: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        let Some((n, rest)) = nodes.split_first() else {
            return k(pos);
        };
        match n {
            Node::Ch(x) => {
                pos < s.len()
                    && self.item_matches(&Item::Ch(*x), s[pos])
                    && self.seq(s, rest, pos + 1, k)
            }
            Node::Any => {
                pos < s.len()
                    && !matches!(s[pos], '\n' | '\r' | '\u{2028}' | '\u{2029}')
                    && self.seq(s, rest, pos + 1, k)
            }
            Node::Class { items, neg } => {
                pos < s.len()
                    && (items.iter().any(|it| self.item_matches(it, s[pos])) != *neg)
                    && self.seq(s, rest, pos + 1, k)
            }
            Node::Start => pos == 0 && self.seq(s, rest, pos, k),
            Node::End => pos == s.len() && self.seq(s, rest, pos, k),
            Node::WordB(want) => {
                let a = pos > 0 && is_word(s[pos - 1]);
                let b = pos < s.len() && is_word(s[pos]);
                ((a != b) == *want) && self.seq(s, rest, pos, k)
            }
            Node::Group(alts) => self.alts(s, alts, pos, &mut |p| self.seq(s, rest, p, k)),
            Node::Repeat {
                alts,
                min,
                max,
                greedy,
            } => self.repeat(s, alts, *min, *max, *greedy, 0, pos, rest, k),
            Node::Look { ahead, neg, alts } => {
                let found = if *ahead {
                    self.alts(s, alts, pos, &mut |_| true)
                } else {
                    // Only as far back as the look-behind can reach: trying every earlier
                    // start made each pattern quadratic in the text (minutes on a 100k record).
                    let from = max_width(alts).map_or(0, |w| pos.saturating_sub(w));
                    (from..=pos)
                        .rev()
                        .any(|j| self.alts(s, alts, j, &mut |p| p == pos))
                };
                found != *neg && self.seq(s, rest, pos, k)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &self,
        s: &[char],
        alts: &[Vec<Node>],
        min: usize,
        max: usize,
        greedy: bool,
        count: usize,
        pos: usize,
        rest: &[Node],
        k: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        let more = |k: &mut dyn FnMut(usize) -> bool| {
            count < max
                && self.alts(s, alts, pos, &mut |p| {
                    // A zero-width iteration past the minimum would loop forever.
                    (p != pos || count < min)
                        && self.repeat(s, alts, min, max, greedy, count + 1, p, rest, k)
                })
        };
        // Not the same block twice: `||` short-circuits, so the order is the greedy (more
        // first) or lazy (rest first) backtracking order. Clippy compares `||` as commutative.
        #[allow(clippy::if_same_then_else)]
        if greedy {
            more(k) || (count >= min && self.seq(s, rest, pos, k))
        } else {
            (count >= min && self.seq(s, rest, pos, k)) || more(k)
        }
    }

    /// The first match at or after char offset `from`: (start, end) in chars.
    pub fn find_at(&self, s: &[char], from: usize) -> Option<(usize, usize)> {
        for start in from..=s.len() {
            let mut end = None;
            if self.alts(s, &self.alts, start, &mut |p| {
                end = Some(p);
                true
            }) {
                return end.map(|e| (start, e));
            }
        }
        None
    }

    pub fn is_match(&self, text: &str) -> bool {
        let s: Vec<char> = text.chars().collect();
        self.find_at(&s, 0).is_some()
    }

    /// `String.prototype.replace` with a global regex and a callback: `f(match, offset, full)`.
    pub fn replace_all(
        &self,
        text: &str,
        mut f: impl FnMut(&str, usize, &str) -> String,
    ) -> String {
        let s: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut cursor = 0;
        let mut from = 0;
        while from <= s.len() {
            let Some((a, b)) = self.find_at(&s, from) else {
                break;
            };
            out.extend(&s[cursor..a]);
            let m: String = s[a..b].iter().collect();
            out.push_str(&f(&m, a, text));
            cursor = b;
            from = if b == a { b + 1 } else { b };
            if b == a && a < s.len() {
                out.push(s[a]);
                cursor = a + 1;
            }
        }
        if cursor < s.len() {
            out.extend(&s[cursor..]);
        }
        out
    }

    pub fn replace_with(&self, text: &str, rep: &str) -> String {
        self.replace_all(text, |_, _, _| rep.to_owned())
    }

    /// All matches as strings.
    pub fn find_all(&self, text: &str) -> Vec<String> {
        let s: Vec<char> = text.chars().collect();
        let mut v = Vec::new();
        let mut from = 0;
        while let Some((a, b)) = self.find_at(&s, from) {
            v.push(s[a..b].iter().collect());
            from = if b == a { b + 1 } else { b };
            if from > s.len() {
                break;
            }
        }
        v
    }
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn upper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// The most chars `alts` can match, if bounded.
fn max_width(alts: &[Vec<Node>]) -> Option<usize> {
    alts.iter().try_fold(0, |widest, seq| {
        let width = seq
            .iter()
            .try_fold(0usize, |sum, node| sum.checked_add(node_width(node)?))?;
        Some(widest.max(width))
    })
}

fn node_width(node: &Node) -> Option<usize> {
    match node {
        Node::Ch(_) | Node::Any | Node::Class { .. } => Some(1),
        Node::Start | Node::End | Node::WordB(_) | Node::Look { .. } => Some(0),
        Node::Group(alts) => max_width(alts),
        Node::Repeat { alts, max, .. } if *max != usize::MAX => max_width(alts)?.checked_mul(*max),
        Node::Repeat { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Regex;

    #[test]
    fn basics() {
        let r = Regex::new(r"\b[0-9a-f]{64,}\b", true).unwrap();
        let tx = "a".repeat(64);
        assert!(r.is_match(&format!("tx {tx} done")));
        assert!(!r.is_match(&format!("tx {}x", "a".repeat(64))));
        let r = Regex::new("foo|ba+r", false).unwrap();
        assert_eq!(r.find_all("foo baaar bar"), vec!["foo", "baaar", "bar"]);
        let r = Regex::new(
            r"(?<![A-Za-z0-9_])sk-[A-Za-z0-9]{6,}(?![A-Za-z0-9_])",
            false,
        )
        .unwrap();
        assert_eq!(r.replace_with("key sk-abcdef123 x", "[s]"), "key [s] x");
        assert_eq!(r.replace_with("ask-abcdef123", "[s]"), "ask-abcdef123");
        // Look-behinds of every width, bounded and not.
        let r = Regex::new(r"(?<=ab|c)x|(?<!q{2,3})y|(?<=z.*)w", false).unwrap();
        assert_eq!(
            r.find_all("abx cx bx qqy qy y zzzw w"),
            vec!["x", "x", "y", "y", "w", "w"]
        );
        let r = Regex::new("a.*?b", false).unwrap();
        assert_eq!(r.find_all("axxbyyb"), vec!["axxb"]);
        assert!(Regex::new("(", false).is_err());
        assert!(Regex::new("[z-a]", false).is_err());
        let r = Regex::new("ERROR", true).unwrap();
        assert!(r.is_match("an error here"));
        let r = Regex::new(r"[€$£¥₿]\s*\d+", false).unwrap();
        assert_eq!(r.find_all("paid €12 and $ 3"), vec!["€12", "$ 3"]);
    }
}
