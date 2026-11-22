//! A bounded model check of reply delivery: every interleaving of sends, dropped tickets, book
//! resets and out-of-order answers, up to [`DEPTH`] steps and [`REQUESTS`] requests, runs against
//! the real `App` and must keep these properties:
//!
//! 1. An answer whose ticket was dropped reaches nobody (dffe84718: an export started before the
//!    dialog closed answered the reopened dialog, and consent given for one book exported
//!    another).
//! 2. A book reset drops the answers of the book's requests still in flight; only callers whose
//!    state survives a reset (`kept_tag`) still hear theirs.
//! 3. Answers with one tag reach the modules in the order the requests were sent, however the
//!    daemon orders them.
//! 4. No answer is delivered twice, and once every request is answered, every one still wanted
//!    has been delivered.

use std::collections::{BTreeSet, HashMap};

use super::tests::{answer, inject_ticket, quiet_app};
use super::{App, Ticket};

const DEPTH: usize = 8;
const REQUESTS: usize = 3;
/// A tag whose requests a book reset orphans, and one it keeps.
const TAGS: [&str; 2] = ["model.book", "reports.diag.model"];

#[derive(Clone, Copy, Debug)]
enum Step {
    Send(usize),
    Drop(usize),
    Answer(usize),
    Reset,
}

struct Run {
    app: App,
    tickets: Vec<Option<Ticket>>,
    ids: Vec<String>,
    tags: Vec<usize>,
    answered: Vec<bool>,
    dropped_at: HashMap<usize, usize>,
    /// Per reset, the book-tag requests it found undelivered.
    orphaned: BTreeSet<usize>,
    delivered: Vec<usize>,
}

impl Run {
    fn new() -> Self {
        Self {
            app: quiet_app(),
            tickets: Vec::new(),
            ids: Vec::new(),
            tags: Vec::new(),
            answered: Vec::new(),
            dropped_at: HashMap::new(),
            orphaned: BTreeSet::new(),
            delivered: Vec::new(),
        }
    }

    fn apply(&mut self, step: Step, at: usize) -> Result<(), String> {
        match step {
            Step::Send(tag) => {
                let kind = format!("model.{}", self.ids.len());
                let (id, ticket) = inject_ticket(&mut self.app, &kind, TAGS[tag]);
                self.ids.push(id);
                self.tickets.push(Some(ticket));
                self.tags.push(tag);
                self.answered.push(false);
            }
            Step::Drop(i) => {
                self.tickets[i] = None;
                self.dropped_at.insert(i, at);
            }
            Step::Answer(i) => {
                self.answered[i] = true;
                answer(
                    &mut self.app,
                    &self.ids[i],
                    &format!("model.{i}"),
                    serde_json::Value::Null,
                );
            }
            Step::Reset => {
                for i in 0..self.ids.len() {
                    if TAGS[self.tags[i]] == "model.book" && !self.delivered.contains(&i) {
                        self.orphaned.insert(i);
                    }
                }
                self.app.reset_book_state();
            }
        }
        self.app.pump();
        let seen: Vec<usize> = self
            .app
            .timings
            .iter()
            .filter_map(|(kind, _)| kind.strip_prefix("model.")?.parse().ok())
            .collect();
        for &i in &seen[self.delivered.len()..] {
            if self.delivered.contains(&i) {
                return Err(format!("request {i} was delivered twice"));
            }
            if self.dropped_at.contains_key(&i) {
                return Err(format!(
                    "request {i} was delivered after its ticket was dropped"
                ));
            }
            if self.orphaned.contains(&i) {
                return Err(format!("request {i} was delivered after a book reset"));
            }
            if let Some(&later) = self
                .delivered
                .iter()
                .find(|&&d| self.tags[d] == self.tags[i] && d > i)
            {
                return Err(format!(
                    "request {i} was delivered after the later request {later}"
                ));
            }
            self.delivered.push(i);
        }
        if self.answered.iter().all(|a| *a) {
            let wanted: Vec<usize> = (0..self.ids.len())
                .filter(|i| self.tickets[*i].is_some() && !self.orphaned.contains(i))
                .filter(|i| !self.delivered.contains(i))
                .collect();
            if !wanted.is_empty() {
                return Err(format!("answered requests {wanted:?} were never delivered"));
            }
        }
        Ok(())
    }
}

/// Steps enabled after `prefix`, without running it: up to [`REQUESTS`] sends, one reset.
fn enabled(prefix: &[Step]) -> Vec<Step> {
    let sent = prefix.iter().filter(|s| matches!(s, Step::Send(_))).count();
    let done = |f: &dyn Fn(&Step) -> bool| prefix.iter().any(f);
    let mut steps = Vec::new();
    if sent < REQUESTS {
        steps.extend((0..TAGS.len()).map(Step::Send));
    }
    for i in 0..sent {
        if !done(&|s| matches!(s, Step::Drop(d) if *d == i)) {
            steps.push(Step::Drop(i));
        }
        if !done(&|s| matches!(s, Step::Answer(a) if *a == i)) {
            steps.push(Step::Answer(i));
        }
    }
    if !done(&|s| matches!(s, Step::Reset)) {
        steps.push(Step::Reset);
    }
    steps
}

/// Runs every maximal step sequence up to `DEPTH` against a fresh `App`, checking the
/// properties after each step (so every prefix is checked too); the first violation is returned
/// with its trace.
fn explore() -> Result<usize, String> {
    fn walk(prefix: &mut Vec<Step>, runs: &mut usize) -> Result<(), String> {
        let next = enabled(prefix);
        if prefix.len() == DEPTH || next.is_empty() {
            *runs += 1;
            let mut run = Run::new();
            for (at, &step) in prefix.iter().enumerate() {
                run.apply(step, at)
                    .map_err(|e| format!("{e}\n  after {:?}", &prefix[..=at]))?;
            }
            return Ok(());
        }
        for step in next {
            prefix.push(step);
            walk(prefix, runs)?;
            prefix.pop();
        }
        Ok(())
    }
    let mut runs = 0;
    walk(&mut Vec::new(), &mut runs)?;
    Ok(runs)
}

#[test]
fn every_interleaving_delivers_answers_only_to_their_live_callers_in_send_order() {
    let runs = explore().unwrap_or_else(|violation| panic!("{violation}"));
    assert!(runs > 10_000, "explored only {runs} sequences");
}
