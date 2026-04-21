import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property string kind: ""
    property real size: 28
    property color inkColor: Design.ink(theme)
    property color paperColor: Design.paper(theme)
    property color accentColor: Design.accent(theme)

    implicitWidth: size
    implicitHeight: size
    width: implicitWidth
    height: implicitHeight

    Loader {
        anchors.fill: parent
        sourceComponent: {
            if (root.kind === "xpub") {
                return xpubIcon
            }
            if (root.kind === "descriptor") {
                return descriptorIcon
            }
            if (root.kind === "core-ln") {
                return coreLnIcon
            }
            if (root.kind === "lnd") {
                return lndIcon
            }
            if (root.kind === "nwc") {
                return nwcIcon
            }
            if (root.kind === "cashu") {
                return cashuIcon
            }
            if (root.kind === "csv") {
                return csvIcon
            }
            return fallbackIcon
        }
    }

    Component {
        id: xpubIcon

        Canvas {
            anchors.fill: parent
            antialiasing: true

            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                ctx.save()
                var scale = Math.min(width, height) / 28.0
                ctx.scale(scale, scale)

                ctx.fillStyle = root.inkColor
                ctx.beginPath()
                ctx.arc(14.0, 14.0, 12.0, 0, Math.PI * 2)
                ctx.fill()

                ctx.strokeStyle = root.paperColor
                ctx.lineWidth = 1.4
                ctx.lineCap = "round"
                ctx.lineJoin = "round"
                ctx.beginPath()
                ctx.moveTo(10.0, 8.0)
                ctx.lineTo(10.0, 20.0)
                ctx.moveTo(10.0, 8.0)
                ctx.lineTo(15.0, 8.0)
                ctx.quadraticCurveTo(17.5, 8.0, 17.5, 11.0)
                ctx.quadraticCurveTo(17.5, 14.0, 15.0, 14.0)
                ctx.lineTo(10.0, 14.0)
                ctx.moveTo(13.0, 14.0)
                ctx.lineTo(17.5, 20.0)
                ctx.stroke()
                ctx.restore()
            }
        }
    }

    Component {
        id: descriptorIcon

        Item {
            Rectangle {
                anchors.fill: parent
                color: root.inkColor
            }

            Repeater {
                model: [
                    { "y": 9, "x": 7, "w": 14 },
                    { "y": 14, "x": 7, "w": 10 },
                    { "y": 19, "x": 7, "w": 7 }
                ]

                Rectangle {
                    x: (modelData["x"] / 28.0) * root.size
                    y: (modelData["y"] / 28.0) * root.size
                    width: (modelData["w"] / 28.0) * root.size
                    height: 1.4
                    color: root.paperColor
                }
            }
        }
    }

    Component {
        id: coreLnIcon

        Canvas {
            anchors.fill: parent
            antialiasing: true

            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                ctx.save()
                var scale = Math.min(width, height) / 28.0
                ctx.scale(scale, scale)

                ctx.fillStyle = root.inkColor
                ctx.beginPath()
                ctx.arc(14.0, 14.0, 12.0, 0, Math.PI * 2)
                ctx.fill()

                ctx.fillStyle = root.accentColor
                ctx.beginPath()
                ctx.moveTo(15.0, 6.0)
                ctx.lineTo(9.0, 16.0)
                ctx.lineTo(13.0, 16.0)
                ctx.lineTo(11.0, 22.0)
                ctx.lineTo(19.0, 11.0)
                ctx.lineTo(15.0, 11.0)
                ctx.closePath()
                ctx.fill()
                ctx.restore()
            }
        }
    }

    Component {
        id: lndIcon

        Item {
            Rectangle {
                anchors.fill: parent
                color: root.inkColor
            }

            Text {
                anchors.centerIn: parent
                text: "lnd"
                color: root.paperColor
                font.family: Design.serif(theme)
                font.pixelSize: Math.max(10, root.size * 0.46)
                font.weight: Font.Bold
            }
        }
    }

    Component {
        id: nwcIcon

        Item {
            Rectangle {
                width: root.size * 0.64
                height: width
                anchors.centerIn: parent
                rotation: 45
                color: root.inkColor
            }

            Rectangle {
                width: root.size * 0.58
                height: width
                anchors.centerIn: parent
                color: root.inkColor
            }

            Canvas {
                anchors.fill: parent
                antialiasing: true

                onPaint: {
                    var ctx = getContext("2d")
                    ctx.clearRect(0, 0, width, height)
                    ctx.save()
                    var scale = Math.min(width, height) / 28.0
                    ctx.scale(scale, scale)
                    ctx.fillStyle = root.accentColor
                    ctx.beginPath()
                    ctx.moveTo(9.0, 14.0)
                    ctx.lineTo(13.0, 10.0)
                    ctx.lineTo(13.0, 14.0)
                    ctx.lineTo(19.0, 14.0)
                    ctx.lineTo(15.0, 18.0)
                    ctx.lineTo(15.0, 14.0)
                    ctx.closePath()
                    ctx.fill()
                    ctx.restore()
                }
            }
        }
    }

    Component {
        id: cashuIcon

        Canvas {
            anchors.fill: parent
            antialiasing: true

            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                ctx.save()
                var scale = Math.min(width, height) / 28.0
                ctx.scale(scale, scale)

                ctx.fillStyle = root.inkColor
                ctx.beginPath()
                ctx.arc(14.0, 14.0, 12.0, 0, Math.PI * 2)
                ctx.fill()

                ctx.strokeStyle = root.accentColor
                ctx.lineWidth = 1.4
                ctx.beginPath()
                ctx.arc(14.0, 14.0, 6.0, 0, Math.PI * 2)
                ctx.stroke()

                ctx.fillStyle = root.accentColor
                ctx.beginPath()
                ctx.arc(14.0, 14.0, 2.0, 0, Math.PI * 2)
                ctx.fill()
                ctx.restore()
            }
        }
    }

    Component {
        id: csvIcon

        Canvas {
            anchors.fill: parent
            antialiasing: true

            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                ctx.save()
                var scale = Math.min(width, height) / 28.0
                ctx.scale(scale, scale)

                ctx.strokeStyle = root.inkColor
                ctx.lineWidth = 1.2
                ctx.strokeRect(2.0, 2.0, 24.0, 24.0)

                ctx.beginPath()
                ctx.moveTo(7.0, 9.0)
                ctx.lineTo(21.0, 9.0)
                ctx.moveTo(7.0, 14.0)
                ctx.lineTo(21.0, 14.0)
                ctx.moveTo(7.0, 19.0)
                ctx.lineTo(21.0, 19.0)
                ctx.moveTo(11.0, 6.0)
                ctx.lineTo(11.0, 22.0)
                ctx.moveTo(17.0, 6.0)
                ctx.lineTo(17.0, 22.0)
                ctx.stroke()
                ctx.restore()
            }
        }
    }

    Component {
        id: fallbackIcon

        Rectangle {
            anchors.fill: parent
            color: root.inkColor
        }
    }
}
