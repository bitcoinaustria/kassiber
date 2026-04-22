# Screen Spec

## Metadata

- Phase:
- Screen:
- Owner:
- Status: draft | in-progress | ready-for-port | wired
- Target viewport:
- Related mockup files:
- Target QML files:

## Purpose

What this screen is for and what user outcome it should enable.

## Frozen references

- Desktop reference:
- Secondary reference:
- Modal/open-state references:
- Source bundle notes:

## Screen states in scope

- [ ] Empty
- [ ] Populated
- [ ] Loading
- [ ] Error
- [ ] Modal open
- [ ] Other:

## Copy lock

Record the exact copy for this pass so wording changes do not get mistaken for
layout drift.

- Title:
- Supporting text:
- CTA labels:
- Table headers / chips / labels:

## Layout tree

Describe the visual hierarchy from the outside in.

Example:

1. App frame
2. Top bar
3. Main content area
4. Left card stack
5. Right utility column

## Tokens used on this screen

Only list the tokens that matter for this screen.

- Fonts:
- Font sizes:
- Spacing:
- Radii:
- Border widths:
- Fill colors:
- Accent colors:
- Shadow treatment:

## Reusable QML components needed

List the components that should exist before or during the port.

- [ ] `Card`
- [ ] `PrimaryButton`
- [ ] `FilterPill`
- [ ] `EmptyState`
- [ ] `ModalShell`
- [ ] Other:

## Static QML pass rules

- mock data only:
- interactions allowed in static pass:
- interactions deferred:
- known visual compromises allowed in pass 1:

## Runtime wiring plan

Only fill in the view-model or runtime surfaces that the final wired screen
will need.

- View-models:
- Signals/actions:
- Data sources:
- Worker-thread needs:

## Acceptance checklist

- [ ] main hierarchy matches the reference
- [ ] spacing rhythm is consistent
- [ ] copy wraps in the right places
- [ ] proportions between cards/columns match
- [ ] reusable components extracted where needed
- [ ] non-goals for this pass are explicit

## Open questions

- 
