---
name: kassiber-desktop-delivery-model
description: Use when a change touches how the native desktop routes daemon answers to screens (App::send, send_ticket, Ticket, deliver_as, deliver, reset_book_state, kept_tag, same-tag parking, stale rereads in crates/kassiber-desktop/src/app.rs), or when a desktop bug looks like an answer reaching a closed dialog, another book, or the wrong record. Runs the bounded model check in src/app/delivery_model.rs and explains how to read and update it.
---

# Desktop delivery model

## What it promises

Every daemon answer reaches only the screen that still wants it, at most once, in the order
its requests were sent. When this breaks, an answer lands in the wrong place: in
dffe84718 an export started before its dialog closed answered the reopened dialog, and
consent given for one book exported another.

`crates/kassiber-desktop/src/app/delivery_model.rs` checks that promise against the real
`App`, not a copy of its rules. It sends requests, drops tickets, resets the book and answers
in every order, then reads which answers the app delivered.

## Does my change touch it?

Yes, when the diff changes any of these in `crates/kassiber-desktop/src/app.rs`:
`App::send`, `App::send_ticket`, `Ticket`, `deliver_as`, `deliver`, `pump`, `on_record`,
`reset_book_state`, `kept_tag`, `earlier_pending`, `release_parked`, `sweep_parked`,
`reread`, or the `tickets` / `delivering_ticket` / `parked` / `pending` fields. Also when a
screen starts or stops holding a `Ticket`, or a tag moves in or out of `kept_tag`.

## Run it

From the repo root, once per checkout (the desktop tests read pinned test data at compile
time):

```sh
scripts/fetch-test-data.sh
```

Then:

```sh
cd crates/kassiber-desktop
mise exec rust@1.99.1 -- cargo test --locked --release delivery_model
```

The test explores about 90,000 step sequences in about 6 s; the release build before it takes
one to two minutes after a change to `app.rs`. Give each checkout its own `CARGO_TARGET_DIR`:
two checkouts of this crate sharing one overwrite each other's test binary. CI runs the
check with the other desktop unit tests in `crates/kassiber-desktop/packaging/ci/rune-checks.sh
--skip-scenes`.

## Read a failure

A failure prints the broken promise and the shortest step list that breaks it, for example

```
request 0 was delivered after its ticket was dropped
  after [Send("model.book"), Drop(0), Answer(0)]
```

Read it as a story. `Send(tag)` sends the next request, numbered from 0, under that tag:
`model.book` is a book-scoped tag a book reset orphans, `reports.diag.model` one that
`kept_tag` keeps. `Drop(i)` is the screen closing and dropping request i's ticket.
`Answer(i)` is the daemon answering request i. `Reset` is a book switch through
`reset_book_state`.

The messages map to the promise:

- `delivered after its ticket was dropped`: a closed or replaced screen heard the answer.
- `delivered after a book reset`: a book-scoped answer reached the next book.
- `delivered after the later request`: same-tag parking let a later answer overtake.
- `delivered twice` or `never delivered`: routing lost or duplicated an answer.

**Real bug or modeling error?** Replay the trace as a unit test in `app.rs`'s `tests`
module: build the app with `tests::quiet_app()`, send with `tests::inject_ticket(&mut app,
kind, tag)`, drop the returned ticket, call `app.reset_book_state()`, answer with
`tests::answer(&mut app, &id, kind, data)`, then `app.pump()`. A delivered answer shows up
in `app.read(kind)` and in `app.timings`. If the app misbehaves there, it is a product bug:
fix `app.rs`, not the model. Change the model only when the change in behavior is intended,
such as a tag joining `kept_tag`, and say so in the PR.

## Update it when behavior changes on purpose

- New delivery rule: add the step to `Step`, enable it in `enabled`, apply it in
  `Run::apply`, and add the check after `pump()`.
- New tag class: add a tag const of that class and send it from `enabled`. More tags or
  requests multiply the sequences; keep the test under about ten seconds in release.
- Then break the rule in `app.rs` on purpose, confirm the model fails with a readable trace,
  and restore it. A check you never saw fail proves nothing.

## Scope

Checked up to `DEPTH = 8` steps, `REQUESTS = 3` requests, two tags, and one reset per
sequence. Answers are successes delivered through the same daemon generation.

Not checked: `stale_context` rereads, daemon restarts (`fail_gen`), the locked-book path,
error answers, streaming and unsolicited events, and screens that still use plain
`App::send` with their own staleness checks. A pass covers these bounds, not the app.

## Coverage record

Last confirmed to describe the code at **d6e9174db**. Covers
`crates/kassiber-desktop/src/app.rs` (the functions listed under "Does my change touch
it?") and `crates/kassiber-desktop/src/app/delivery_model.rs`.

To see whether that still holds:

```sh
git log --oneline d6e9174db..HEAD -- crates/kassiber-desktop/src/app.rs crates/kassiber-desktop/src/app/delivery_model.rs
```

When commits show up, read their delivery changes. Move the commit above forward only after
you confirm the model still matches the code at the new commit, and after the test passes
there.
