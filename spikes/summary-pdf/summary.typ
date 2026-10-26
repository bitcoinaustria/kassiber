#let r = json(bytes(sys.inputs.data))
#set document(title: r.title, author: "Kassiber", date: datetime(year: 2026, month: 5, day: 1, hour: 12, minute: 0, second: 0))
#set text(font: ("Bitstream Vera Sans", "Liberation Sans"), size: 8pt, fill: rgb("#222222"))
#set par(leading: 3pt)
#set page(paper: "a4", margin: (left: 14mm, right: 14mm, top: 18mm, bottom: 14mm),
  header: [#grid(columns: (1fr, 1fr), strong("Kassiber"), align(right)[#text(size: 7pt)[Summary report]])#line(length: 100%, stroke: 0.4pt + rgb("#d9d9d9"))],
  footer: context [#grid(columns: (1fr, auto), text(size: 7pt)[#r.book · #r.timeframe], text(size: 7pt)[Page #counter(page).display() of #counter(page).final().first()])])
#let section(title) = block(sticky: true, above: 5pt, below: 4pt, text(size: 12.5pt, weight: "bold", title))
#let muted(value) = text(size: 8pt, fill: rgb("#666666"), value)
#let report-table(data, columns, right-cols: ()) = {
  set text(size: 7pt)
  table(columns: columns, inset: 4pt, stroke: (bottom: 0.3pt + rgb("#d9d9d9")),
    fill: (x,y) => if y == 0 {rgb("#f7f7f7")} else if calc.even(y) {rgb("#fbfbfb")},
    align: (x,y) => if right-cols.contains(x) {right} else {left},
    table.header(..data.first().map(cell => strong(cell))),
    ..data.slice(1).flatten())
}
#let chart(c, ring: false) = box(width: 180mm, height: c.height * 1pt)[
  #if ring {place(top + left, image("ring.svg", width: 180mm))}
  #for l in c.lines {place(top + left, line(start: (l.x1*1pt,l.y1*1pt), end: (l.x2*1pt,l.y2*1pt), stroke: l.width*1pt + rgb(l.color)))}
  #for r in c.rects {place(top + left, dx:r.x*1pt, dy:r.y*1pt, rect(width:r.w*1pt,height:r.h*1pt,fill:rgb(r.color),stroke:none))}
  #for p in c.circles {place(top + left, dx:p.x*1pt,dy:p.y*1pt,circle(radius:p.r*1pt,fill:white,stroke:0.8pt+rgb(p.color)))}
  #for t in c.labels {place(top + left, dx:t.x*1pt,dy:t.y*1pt,box(width:t.width*1pt,align(if t.align == "right" {right} else if t.align == "center" {center} else {left},text(size:t.size*1pt,weight:if t.bold {"bold"}else{"regular"},fill:rgb(t.color),t.text))))}
]
#text(size: 23pt, weight: "bold", r.title)

#r.book

#muted("Timeframe: " + r.timeframe + " · Generated: " + r.generated)
#if r.snapshot.len() > 1 {
  section(r.snapshot_heading)
  report-table(r.snapshot, (44mm,34mm,38mm,45mm), right-cols:(2,3))
}
#v(6pt)
#grid(columns: (1fr,1fr,1fr,1fr,1fr), gutter: 4pt,
  ..r.kpis.map(k => block(fill:rgb("#f7f7f7"),inset:5pt)[#text(size:7pt,k.label) \ #text(size:10pt,weight:"bold",k.value)]))
#for p in r.performance {v(2pt); muted(p)}
#section("Data Integrity")
#report-table(r.integrity, (70mm,92mm))
#section("Portfolio Movement")
#chart(r.movement_chart)
#pagebreak()
#section("Portfolio Composition")
#chart(r.composition_chart, ring:true)
#muted(r.age)
#section("Period Activity")
#chart(r.pnl_chart)
#if r.disposals.len() > 1 {v(4pt);text("Largest disposals");v(3pt);report-table(r.disposals,(22mm,36mm,30mm,30mm,30mm,34mm),right-cols:(2,3,4,5))}
#v(7pt)
#chart(r.flow_chart)
#if r.movements.len() > 1 {v(4pt);text("Largest activity");v(3pt);report-table(r.movements,(22mm,30mm,12mm,14mm,28mm,30mm,46mm),right-cols:(4,5))}
#section("Wallet Appendix")
#report-table(r.appendix,(42mm,38mm,22mm,38mm,42mm),right-cols:(2,3,4))
#v(8pt)
#muted("This summary report is a portfolio and treasury view. It intentionally omits tax tables; use the tax PDF for tax filing support.")
