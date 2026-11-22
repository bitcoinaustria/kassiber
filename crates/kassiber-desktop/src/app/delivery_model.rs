use std::collections::{BTreeSet, HashMap};

use super::tests::{answer, inject_ticket, quiet_app};
use super::{App, Ticket};

const DEPTH: usize = 8;
const REQUESTS: usize = 3;
const BOOK_TAG: &str = "model.book";
const KEPT_TAG: &str = "reports.diag.model";

#[derive(Clone, Copy, Debug)]
enum Step {
    Send(&'static str),
    Drop(usize),
    Answer(usize),
    Reset,
}

struct Run {
    app: App,
    tickets: Vec<Option<Ticket>>,
    ids: Vec<String>,
    tags: Vec<&'static str>,
    answered: Vec<bool>,
    dropped_at: HashMap<usize, usize>,
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
                let (id, ticket) = inject_ticket(&mut self.app, &kind, tag);
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
                    if self.tags[i] == BOOK_TAG && !self.delivered.contains(&i) {
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

fn enabled(prefix: &[Step]) -> Vec<Step> {
    let sent = prefix.iter().filter(|s| matches!(s, Step::Send(_))).count();
    let done = |f: &dyn Fn(&Step) -> bool| prefix.iter().any(f);
    let mut steps = Vec::new();
    if sent < REQUESTS {
        steps.extend([BOOK_TAG, KEPT_TAG].map(Step::Send));
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

fn explore() -> Result<usize, String> {
    struct Search {
        runs: usize,
        shortest: Option<(usize, String)>,
    }
    fn walk(prefix: &mut Vec<Step>, search: &mut Search) {
        let next = enabled(prefix);
        if prefix.len() == DEPTH || next.is_empty() {
            search.runs += 1;
            let mut run = Run::new();
            for (at, &step) in prefix.iter().enumerate() {
                if let Err(e) = run.apply(step, at) {
                    if search
                        .shortest
                        .as_ref()
                        .is_none_or(|(len, _)| at + 1 < *len)
                    {
                        search.shortest =
                            Some((at + 1, format!("{e}\n  after {:?}", &prefix[..=at])));
                    }
                    return;
                }
            }
            return;
        }
        for step in next {
            prefix.push(step);
            walk(prefix, search);
            prefix.pop();
        }
    }
    let mut search = Search {
        runs: 0,
        shortest: None,
    };
    walk(&mut Vec::new(), &mut search);
    match search.shortest {
        Some((_, violation)) => Err(violation),
        None => Ok(search.runs),
    }
}

#[test]
fn every_interleaving_delivers_answers_only_to_their_live_callers_in_send_order() {
    let runs = explore().unwrap_or_else(|violation| panic!("{violation}"));
    assert!(runs > 10_000, "explored only {runs} sequences");
}
