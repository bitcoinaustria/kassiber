import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property real size: 28
    property color strokeColor: Design.ink(theme)
    property color sealColor: Design.accent(theme)

    implicitWidth: size
    implicitHeight: size
    width: implicitWidth
    height: implicitHeight

    Canvas {
        id: canvas
        anchors.fill: parent
        antialiasing: true

        function roundRect(ctx, x, y, width, height, radius) {
            ctx.beginPath()
            ctx.moveTo(x + radius, y)
            ctx.lineTo(x + width - radius, y)
            ctx.quadraticCurveTo(x + width, y, x + width, y + radius)
            ctx.lineTo(x + width, y + height - radius)
            ctx.quadraticCurveTo(x + width, y + height, x + width - radius, y + height)
            ctx.lineTo(x + radius, y + height)
            ctx.quadraticCurveTo(x, y + height, x, y + height - radius)
            ctx.lineTo(x, y + radius)
            ctx.quadraticCurveTo(x, y, x + radius, y)
            ctx.closePath()
        }

        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            ctx.save()
            var scale = Math.min(width, height) / 32.0
            ctx.scale(scale, scale)

            ctx.strokeStyle = root.strokeColor
            ctx.lineWidth = 1.25
            roundRect(ctx, 3.5, 6.5, 25.0, 19.0, 1.0)
            ctx.stroke()

            ctx.beginPath()
            ctx.moveTo(3.5, 6.5)
            ctx.lineTo(16.0, 17.0)
            ctx.lineTo(28.5, 6.5)
            ctx.stroke()

            ctx.beginPath()
            ctx.moveTo(3.5, 25.5)
            ctx.lineTo(12.0, 17.0)
            ctx.moveTo(28.5, 25.5)
            ctx.lineTo(20.0, 17.0)
            ctx.stroke()

            ctx.beginPath()
            ctx.fillStyle = root.sealColor
            ctx.arc(16.0, 20.0, 2.2, 0, Math.PI * 2)
            ctx.fill()
            ctx.lineWidth = 0.75
            ctx.strokeStyle = root.strokeColor
            ctx.stroke()
            ctx.restore()
        }
    }

    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()
    onStrokeColorChanged: canvas.requestPaint()
    onSealColorChanged: canvas.requestPaint()
}
