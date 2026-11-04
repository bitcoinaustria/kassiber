// Kassiber's shared report design: one calm layout for every report kind,
// country and section. The Rust side hands over a finished document with
// every string formatted; this template only places it.

#let ink = rgb("#1f2328")
#let muted = rgb("#5b6169")
#let hairline = rgb("#d0d4d9")
#let accent = rgb("#c8102e")

// Every bundled face is in both lists so any character the engine admitted
// (it replaces the rest with U+FFFD) finds a glyph.
#let sans = ("DM Sans", "Liberation Sans", "JetBrains Mono")
#let mono = ("JetBrains Mono", "Liberation Sans", "DM Sans")

#let tone-fill(tone) = if tone == "accent" { accent } else if tone == "muted" { muted } else { ink }

// Amount columns use DM Sans's tabular figures so they line up. Its `tnum`
// also widens the separators, so only digit runs take it; prose and single
// figures stay proportional. Only identifiers (codes, ids, hashes) use the
// mono face.
#let cell-text(value, mono-face: false, tabular: false, size: 8pt, weight: "regular", fill: ink) = {
  if mono-face {
    text(font: mono, size: size * 0.92, weight: weight, fill: fill, value)
  } else if tabular {
    show regex("[0-9]+"): set text(number-width: "tabular")
    text(font: sans, size: size, weight: weight, fill: fill, value)
  } else {
    text(font: sans, size: size, weight: weight, fill: fill, value)
  }
}

// Label and value pairs in two quiet columns.
#let facts(block) = grid(
  columns: (38%, 1fr),
  column-gutter: 12pt,
  row-gutter: 5pt,
  ..block.rows.map(row => (
    text(size: 8pt, fill: muted, row.label),
    cell-text(row.value, size: 8.5pt, fill: tone-fill(row.tone)),
  )).flatten(),
)

// Headline figures: label above value, four to a row, no boxes.
#let kpis(block) = grid(
  columns: (1fr,) * 4,
  column-gutter: 14pt,
  row-gutter: 12pt,
  ..block.items.map(item => stack(
    spacing: 4pt,
    text(size: 7.5pt, fill: muted, item.label),
    cell-text(item.value, size: 12pt, weight: 500, fill: tone-fill(item.tone)),
    if item.detail != "" { text(size: 7pt, fill: muted, item.detail) },
  )),
)

// A table is its header, a hairline under it and right-aligned figures.
#let report-table(block) = {
  if block.title != "" { std.block(sticky: true, below: 4pt, text(size: 8pt, weight: 500, block.title)) }
  let cols = block.columns
  let aligns = cols.map(c => if c.align == "right" { right } else { left })
  let header = cols.map(c => text(size: 7pt, weight: 500, fill: muted, hyphenate: true, c.label))
  let body = block.rows.map(row => row.enumerate().map(((i, value)) =>
    cell-text(value, mono-face: cols.at(i).mono, tabular: cols.at(i).align == "right"))).flatten()
  let total = if block.total == none { () } else {
    block.total.enumerate().map(((i, value)) =>
      cell-text(value, mono-face: cols.at(i).mono, tabular: cols.at(i).align == "right", weight: 500))
  }
  table(
    columns: cols.map(c => if c.width == "auto" { auto } else { 1fr * c.width }),
    align: (x, y) => aligns.at(x) + top,
    stroke: (x, y) => (
      bottom: if y == 0 { 0.5pt + ink } else { none },
      top: if block.total != none and y == block.rows.len() + 1 { 0.4pt + hairline } else { none },
    ),
    inset: (x: 4pt, y: 3.2pt),
    table.header(..header),
    ..body,
    ..total,
  )
  if block.note != "" { v(3pt); text(size: 7pt, fill: muted, block.note) }
}

#let note(block) = {
  let marker = if block.tone == "accent" { text(fill: accent, weight: 500, "! ") } else { [] }
  par(text(size: 7.5pt, fill: if block.tone == "accent" { ink } else { muted }, marker + block.text))
}

// Charts arrive in unit coordinates (0..1, y up) and are scaled here.
#let chart(block) = {
  text(size: 8pt, weight: 500, block.title)
  v(4pt)
  let height = block.height * 1pt
  layout(size => {
    let left-gutter = 46pt
    let right-gutter = if block.y2_labels.len() > 0 { 40pt } else { 6pt }
    let w = size.width - left-gutter - right-gutter
    let px(x) = left-gutter + w * x
    let py(y) = height * (1 - y)
    box(width: 100%, height: height + 14pt, {
      for tick in block.y_labels {
        place(top + left, line(start: (left-gutter, py(tick.pos)), end: (left-gutter + w, py(tick.pos)),
          stroke: 0.3pt + hairline))
        place(top + left, dx: 0pt, dy: py(tick.pos) - 4pt,
          box(width: left-gutter - 6pt, align(right, cell-text(tick.text, size: 6.2pt, fill: muted))))
      }
      for tick in block.y2_labels {
        place(top + left, dx: left-gutter + w + 6pt, dy: py(tick.pos) - 4pt,
          cell-text(tick.text, size: 6.2pt, fill: muted))
      }
      for tick in block.x_labels {
        place(top + left, dx: px(tick.pos) - 20pt, dy: height + 4pt,
          box(width: 40pt, align(center, cell-text(tick.text, size: 6pt, fill: muted))))
      }
      for s in block.series {
        let paint = if s.tone == "accent" { accent } else if s.tone == "muted" { muted } else { ink }
        if s.kind == "line" {
          let stroke = (paint: paint, thickness: s.weight * 1pt,
            dash: if s.dashed { (2pt, 1.5pt) } else { none })
          let pts = s.points.map(p => (px(p.at(0)), py(p.at(1))))
          if pts.len() > 1 {
            place(top + left, curve(stroke: stroke, curve.move(pts.first()), ..pts.slice(1).map(p => curve.line(p))))
          }
        } else {
          for b in s.bars {
            let top-y = calc.max(b.at(2), b.at(3))
            let bottom-y = calc.min(b.at(2), b.at(3))
            place(top + left, dx: px(b.at(0)), dy: py(top-y),
              rect(width: w * (b.at(1) - b.at(0)), height: height * (top-y - bottom-y),
                fill: if s.hollow { none } else { paint },
                stroke: if s.hollow { 0.5pt + paint } else { none }))
          }
        }
      }
    })
  })
  if block.legend.len() > 0 {
    v(2pt)
    block.legend.map(item => {
      let paint = tone-fill(item.tone)
      let mark = if item.mark == "filled" {
        box(width: 7pt, height: 6pt, fill: paint)
      } else if item.mark == "hollow" {
        box(width: 7pt, height: 6pt, stroke: 0.5pt + paint)
      } else {
        let weight = if item.mark == "line" { 1.4pt } else if item.mark == "thin" { 0.6pt } else { 0.8pt }
        box(width: 14pt, height: 6pt, align(horizon, line(length: 14pt, stroke: (paint: paint, thickness: weight,
          dash: if item.mark == "dashed" { (2pt, 1.5pt) } else { none }))))
      }
      box[#mark #h(3pt) #text(size: 6.8pt, fill: muted, item.label)]
    }).join(h(12pt))
  }
  v(6pt)
}

#let section(s) = {
  block(above: 16pt, below: 8pt, sticky: true, {
    text(size: 11pt, weight: 500, s.title)
    if s.lead != "" { v(2pt); text(size: 7.5pt, fill: muted, s.lead) }
  })
  for b in s.blocks {
    block(above: 7pt, below: 7pt, breakable: b.t == "table", {
      if b.t == "facts" { facts(b) }
      else if b.t == "kpis" { kpis(b) }
      else if b.t == "table" { report-table(b) }
      else if b.t == "chart" { chart(b) }
      else if b.t == "note" { note(b) }
    })
  }
}

#let report(data) = {
  let m = data.meta
  set document(title: m.title, author: "Kassiber", keywords: m.keywords,
    date: datetime(year: m.date.at(0), month: m.date.at(1), day: m.date.at(2),
      hour: m.date.at(3), minute: m.date.at(4), second: m.date.at(5)))
  set text(font: sans, size: 8.5pt, fill: ink, lang: m.lang)
  set par(leading: 0.55em, spacing: 0.9em)
  set page(
    paper: "a4",
    margin: (x: 18mm, top: 22mm, bottom: 20mm),
    header: context {
      if counter(page).get().first() > 1 {
        set text(size: 7pt, fill: muted)
        grid(columns: (1fr, auto), m.running_title, m.book + "  ·  " + m.period)
      }
    },
    footer: context {
      set text(size: 7pt, fill: muted)
      grid(columns: (1fr, auto),
        m.footer,
        m.page_format.replace("{page}", str(counter(page).get().first()))
          .replace("{total}", str(counter(page).final().first())))
    },
  )
  text(size: 7.5pt, fill: muted, m.kicker)
  v(2pt)
  text(size: 20pt, weight: 500, m.title)
  v(1pt)
  text(size: 9pt, fill: muted, m.book + "  ·  " + m.period)
  for s in data.sections {
    if s.at("page_break", default: false) { pagebreak(weak: true) }
    section(s)
  }
}
