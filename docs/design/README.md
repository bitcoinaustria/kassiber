# Design-to-QML Workflow

Use this workflow when porting Claude Design exports or other browser-style
mockups into Kassiber's `PySide6 + QML` desktop UI.

The goal is to improve translation accuracy by treating exported `.jsx` /
HTML as **design evidence**, not implementation source. QML should be written
from a frozen visual spec, not generated directly from React-flavored code.

## Why this exists

Direct JSX-to-QML translation tends to lose fidelity because the source files
mix together:

- visual intent
- React state and event plumbing
- browser-only layout patterns
- inline style details that do not map 1:1 to Qt

Kassiber's better path is:

`JSX mockup -> spec markdown -> static QML -> screenshot review -> runtime wiring`

That keeps layout work, component extraction, and data binding separate so the
model is solving one problem at a time.

## Repo layout

Recommended per-screen artifact layout:

```text
docs/design/
  README.md
  templates/
    screen-spec.md
    screenshot-review.md
  phase-2/
    overview/
      screen-spec.md
      screenshot-review.md
      refs/
        desktop-1280.png
        narrow-1024.png
        modal-settings.png
      source/
        exported-files.txt
        notes.md
```

Notes:

- `refs/` holds the frozen screenshots that define the visual contract.
- `source/` is optional. Use it for notes about the original Claude Design
  export, source file names, or an external path to the `.jsx` bundle.
- Do not depend on an ephemeral Downloads path as the only reference. Copy the
  final screenshots you want to match into the repo under `refs/`.

## Workflow

### 1. Freeze the reference

Before touching QML, choose the exact screen states you want to port.

Required inputs for each screen:

- one desktop screenshot at the target width
- one secondary screenshot if the layout changes materially
- one screenshot per important modal/open state
- the exact copy and mock data used in the mockup
- a note listing the relevant exported `.jsx` files

Good default widths for desktop work:

- `1280px` main desktop view
- `1024px` narrower desktop view when wrapping behavior matters

If the Claude Design export is only available as `.jsx`, render it once, take
screenshots, and treat those screenshots as the visual source of truth.

### 2. Write the screen spec

Create `screen-spec.md` from the template before any QML edits.

The spec should lock down:

- screen purpose
- reference screenshots and viewport sizes
- visual states covered by this pass
- layout tree and relative sizing
- typography, spacing, border, and radius tokens actually used
- reusable QML components needed
- target QML files for the port
- explicit non-goals for the first pass

This is the step that turns "cool mockup" into "something a QML implementer can
reproduce without guessing."

### 3. Build a static QML pass

The first QML pass should be **static**:

- use mock data only
- no real `kassiber.core` calls
- no workers
- no persistence
- no screen-to-screen navigation unless it affects layout accuracy

The point is visual parity, not functionality.

If a design introduces a reusable primitive that appears across multiple
screens, extract it into `kassiber/ui/resources/qml/components/` first.

Examples:

- `Card`
- `PrimaryButton`
- `FilterPill`
- `SectionHeader`
- `EmptyState`
- `ModalShell`

### 4. Run a screenshot review

After the static pass renders, capture the current QML output and compare it
against the frozen reference screenshots.

Create `screenshot-review.md` from the template and record:

- the reference screenshot
- the QML screenshot under review
- mismatches
- whether each mismatch is blocking, accepted, or deferred

Fix the biggest visual mismatches first:

- wrong proportions
- wrong spacing rhythm
- incorrect hierarchy or grouping
- copy wrapping differences
- missing or extra controls

Only after those are stable should you spend time on lower-value polish.

### 5. Wire runtime behavior

Once the screen is visually close enough:

- connect view-model properties
- replace mock data
- add signals and actions
- move long-running work off the UI thread where required

Do not mix visual layout corrections with business-logic wiring unless the
screen is already stable. That is where translation accuracy usually falls
apart.

## Porting rules

When turning mockups into QML:

- treat `.jsx` as reference, not as code to translate line by line
- preserve copy, hierarchy, and relative proportions before styling details
- prefer Qt Quick Controls primitives over custom drawing unless the design
  really needs more
- freeze one screen at a time; avoid porting the whole prototype bundle in one
  pass
- keep the first implementation static until screenshot review is complete
- accept minor font-rendering differences, but do not accept layout drift
- record any intentional deviation in `screenshot-review.md`

## Prompt shape for agents

When using an agent to port a screen, a prompt shaped like this tends to work
better than "convert JSX to QML":

```text
Use the JSX files only as visual reference, not as implementation source.

First:
1. extract the layout tree
2. extract the tokens actually used on this screen
3. list reusable QML components required
4. write a static QML pass with mock data only

Constraints:
- preserve copy, hierarchy, and proportions
- do not redesign
- target the frozen reference screenshots
- prefer Qt Quick Controls primitives
- no business logic yet
```

## Definition of done

A screen is ready to wire into the runtime when:

- the spec is filled in
- the static QML version matches the reference well enough in screenshot review
- accepted deviations are written down
- reusable components are extracted where appropriate
- the remaining work is behavioral, not visual

## Helper command

Use the scaffold script to create a new screen workspace:

```bash
./scripts/scaffold-design-screen.sh phase-2 overview
```

That creates the recommended folder structure and copies in the markdown
templates.
