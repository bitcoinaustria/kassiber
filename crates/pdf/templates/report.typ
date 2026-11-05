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
    // Identifiers (hashes, ids) may break anywhere rather than overflow.
    text(font: mono, size: size * 0.92, weight: weight, fill: fill, value.clusters().join("\u{200b}"))
  } else if tabular {
    show regex("[0-9]+"): set text(number-width: "tabular")
    text(font: sans, size: size, weight: weight, fill: fill, value)
  } else {
    text(font: sans, size: size, weight: weight, fill: fill, value)
  }
}

// Label and value pairs in two quiet columns.
// A long unbroken value (a hash, an id) may break anywhere.
#let breakable(value) = if value.len() > 40 and not value.contains(" ") {
  value.clusters().join("\u{200b}")
} else { value }

#let facts(block) = grid(
  columns: (38%, 1fr),
  column-gutter: 12pt,
  row-gutter: 5pt,
  ..block.rows.map(row => (
    text(size: 8pt, fill: muted, row.label),
    cell-text(breakable(row.value), size: 8.5pt, fill: tone-fill(row.tone)),
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
    // The totals label is prose even above an identifier column.
    block.total.enumerate().map(((i, value)) =>
      cell-text(value, mono-face: i > 0 and cols.at(i).mono, tabular: cols.at(i).align == "right", weight: 500))
  }
  // A row never splits across pages.
  set table.cell(breakable: false)
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

// The legend swatches charts and flow diagrams share.
#let swatch(item) = {
  let paint = tone-fill(item.tone)
  if item.mark == "filled" {
    box(width: 7pt, height: 6pt, fill: paint)
  } else if item.mark == "hollow" {
    box(width: 7pt, height: 6pt, stroke: 0.5pt + paint)
  } else if item.mark == "box" {
    box(width: 9pt, height: 6pt, stroke: 0.5pt + paint, radius: 1pt)
  } else if item.mark == "box-strong" {
    box(width: 9pt, height: 6pt, stroke: 1.1pt + paint, radius: 1pt)
  } else if item.mark == "box-dashed" {
    box(width: 9pt, height: 6pt, stroke: (paint: paint, thickness: 0.6pt, dash: (2pt, 1.5pt)), radius: 1pt)
  } else {
    let weight = if item.mark == "line" { 1.4pt } else if item.mark == "thin" { 0.6pt } else { 0.8pt }
    box(width: 14pt, height: 6pt, align(horizon, line(length: 14pt, stroke: (paint: paint, thickness: weight,
      dash: if item.mark == "dashed" { (2pt, 1.5pt) } else { none }))))
  }
}

#let legend(items) = {
  if items.len() > 0 {
    v(2pt)
    items.map(item => box[#swatch(item) #h(3pt) #text(size: 6.8pt, fill: muted, item.label)]).join(h(12pt))
  }
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
  legend(block.legend)
  v(6pt)
}

// A flow diagram: columns of boxes left to right, links as straight arrows.
// Emphasis is by stroke weight and dash, never colour.
#let flow(block) = layout(size => {
  let cols = block.columns
  let n = calc.max(1, cols.len())
  // Boxes stay narrow; spare width goes to the gaps the arrows cross.
  let w = calc.min((size.width - 16pt * (n - 1)) / n, 150pt)
  let gap = if n > 1 { (size.width - w * n) / (n - 1) } else { 0pt }
  let node-h = 36pt
  let row-gap = 8pt
  let rows = calc.max(1, ..cols.map(c => c.nodes.len()))
  let body-h = rows * node-h + (rows - 1) * row-gap
  let head = 12pt
  let pos = (:)
  for (ci, c) in cols.enumerate() {
    let count = c.nodes.len()
    let group-h = count * node-h + calc.max(0, count - 1) * row-gap
    let y0 = head + (body-h - group-h) / 2
    for (ri, nd) in c.nodes.enumerate() {
      pos.insert(nd.id, (x: ci * (w + gap), y: y0 + ri * (node-h + row-gap)))
    }
  }
  box(width: 100%, height: head + body-h + 2pt, {
    for (ci, c) in cols.enumerate() {
      place(top + left, dx: ci * (w + gap),
        box(width: w, align(center, text(size: 6pt, weight: 500, fill: muted, upper(c.title)))))
    }
    for l in block.links {
      if l.from in pos and l.to in pos {
        let a = pos.at(l.from)
        let b = pos.at(l.to)
        if a.x != b.x {
          let (sx, ex, barb) = if a.x > b.x { (a.x, b.x + w, 4pt) } else { (a.x + w, b.x, -4pt) }
          let sy = a.y + node-h / 2
          let ey = b.y + node-h / 2
          let paint = if l.style == "plain" { ink } else { muted }
          let weight = l.weight * 1pt
          place(top + left, line(start: (sx, sy), end: (ex, ey),
            stroke: (paint: paint, thickness: weight, dash: if l.style == "privacy" { (2pt, 1.5pt) } else { none })))
          place(top + left, line(start: (ex, ey), end: (ex + barb, ey - 2.5pt), stroke: (paint: paint, thickness: weight)))
          place(top + left, line(start: (ex, ey), end: (ex + barb, ey + 2.5pt), stroke: (paint: paint, thickness: weight)))
          if l.label != "" {
            place(top + left, dx: (sx + ex) / 2 - 16pt, dy: (sy + ey) / 2 - 5pt,
              box(width: 32pt, align(center, box(fill: white, inset: 1pt,
                cell-text(l.label, size: 5.6pt, fill: muted)))))
          }
        }
      }
    }
    for c in cols {
      for nd in c.nodes {
        let p = pos.at(nd.id)
        let stroke = if nd.style == "target" { 1.1pt + ink }
          else if nd.style == "source" { 0.8pt + ink }
          else if nd.style in ("attested", "privacy", "omitted") { (paint: muted, thickness: 0.6pt, dash: (2pt, 1.5pt)) }
          else { 0.5pt + muted }
        place(top + left, dx: p.x, dy: p.y,
          std.block(width: w, height: node-h, fill: white, stroke: stroke, radius: 2pt, inset: 4pt, clip: true, {
            set par(leading: 0.35em, spacing: 0.35em)
            text(size: 6.6pt, weight: 500, hyphenate: false, nd.label)
            if nd.detail != "" { linebreak(); text(size: 5.8pt, fill: muted, nd.detail) }
            if nd.amount != "" { linebreak(); cell-text(nd.amount, size: 5.8pt, tabular: true) }
          }))
      }
    }
  })
  legend(block.legend)
})

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
      else if b.t == "flow" { flow(b) }
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
    if s.at("landscape", default: false) {
      page(flipped: true, margin: (x: 14mm, top: 20mm, bottom: 18mm), section(s))
    } else {
      if s.at("page_break", default: false) { pagebreak(weak: true) }
      section(s)
    }
  }
}
