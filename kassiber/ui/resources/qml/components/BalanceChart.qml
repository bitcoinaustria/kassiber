import QtQuick 2.15

import "Design.js" as Design

Canvas {
    id: root
    property var series: []
    property string currency: "btc"
    property real priceEur: 71420.18
    property int padTop: 14
    property int padRight: 14
    property int padBottom: 22
    property int padLeft: 36
    property color lineColor: Design.accent(theme)
    property color gridColor: Design.line(theme)
    property color textColor: Design.ink3(theme)
    property var xLabels: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"]

    implicitHeight: 160
    renderStrategy: Canvas.Immediate
    antialiasing: true

    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onSeriesChanged: requestPaint()
    onCurrencyChanged: requestPaint()

    onPaint: {
        var ctx = getContext("2d")
        ctx.reset()
        if (!series || series.length === 0) return
        var vals = series
        if (currency === "eur") {
            vals = series.map(function(v) { return v * priceEur })
        }
        var maxVal = 0
        for (var i = 0; i < vals.length; i++) {
            if (vals[i] > maxVal) maxVal = vals[i]
        }
        maxVal = maxVal * 1.15
        if (maxVal <= 0) maxVal = 1

        var w = width
        var h = height
        var innerW = w - padLeft - padRight
        var innerH = h - padTop - padBottom
        var stepX = innerW / Math.max(1, vals.length - 1)

        ctx.font = "9px " + Design.mono(theme)
        ctx.fillStyle = textColor

        // y-grid
        var yTicks = [0, 0.25, 0.5, 0.75, 1]
        ctx.lineWidth = 1
        ctx.strokeStyle = gridColor
        for (var t = 0; t < yTicks.length; t++) {
            var yy = padTop + yTicks[t] * innerH
            ctx.beginPath()
            if (t === yTicks.length - 1) {
                ctx.setLineDash([])
            } else {
                ctx.setLineDash([2, 3])
            }
            ctx.moveTo(padLeft, yy)
            ctx.lineTo(w - padRight, yy)
            ctx.stroke()
            var v = (1 - yTicks[t]) * maxVal
            var label
            if (currency === "eur") {
                label = "\u20ac" + Math.round(v).toLocaleString(Qt.locale("de-AT"), 'f', 0)
            } else {
                label = v.toFixed(1)
            }
            ctx.textAlign = "right"
            ctx.textBaseline = "middle"
            ctx.fillText(label, padLeft - 6, yy)
        }
        ctx.setLineDash([])

        // line + area
        var pts = []
        for (var k = 0; k < vals.length; k++) {
            var px = padLeft + k * stepX
            var py = padTop + (1 - vals[k] / maxVal) * innerH
            pts.push([px, py])
        }

        // area fill
        ctx.beginPath()
        ctx.moveTo(pts[0][0], pts[0][1])
        for (var m = 1; m < pts.length; m++) {
            ctx.lineTo(pts[m][0], pts[m][1])
        }
        ctx.lineTo(pts[pts.length - 1][0], h - padBottom)
        ctx.lineTo(pts[0][0], h - padBottom)
        ctx.closePath()
        ctx.fillStyle = Qt.rgba(
            Number("0x" + Design.accent(theme).toString().substr(1, 2)) / 255,
            Number("0x" + Design.accent(theme).toString().substr(3, 2)) / 255,
            Number("0x" + Design.accent(theme).toString().substr(5, 2)) / 255,
            0.08
        )
        ctx.fill()

        // line stroke
        ctx.beginPath()
        ctx.moveTo(pts[0][0], pts[0][1])
        for (var n = 1; n < pts.length; n++) {
            ctx.lineTo(pts[n][0], pts[n][1])
        }
        ctx.strokeStyle = lineColor
        ctx.lineWidth = 1.5
        ctx.stroke()

        // dots
        var showDotEvery = pts.length <= 12 ? 1 : Math.ceil(pts.length / 12)
        for (var d = 0; d < pts.length; d++) {
            if (d % showDotEvery === 0 || d === pts.length - 1) {
                ctx.beginPath()
                ctx.arc(pts[d][0], pts[d][1], 2, 0, Math.PI * 2)
                ctx.fillStyle = Design.paperAlt(theme)
                ctx.fill()
                ctx.strokeStyle = lineColor
                ctx.lineWidth = 1
                ctx.stroke()
            }
        }

        // x labels
        ctx.fillStyle = textColor
        ctx.textAlign = "center"
        ctx.textBaseline = "alphabetic"
        for (var x = 0; x < xLabels.length; x++) {
            var frac = xLabels.length === 1 ? 0.5 : x / (xLabels.length - 1)
            var lx = padLeft + frac * innerW
            ctx.fillText(xLabels[x], lx, h - 6)
        }

        // currency glyph upper-left inside chart
        ctx.textAlign = "left"
        ctx.fillText(currency === "eur" ? "\u20ac" : "\u20bf", padLeft - 28, padTop + 4)
    }
}
