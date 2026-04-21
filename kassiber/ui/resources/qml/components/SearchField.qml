import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

// Compact search input — leading magnifier glyph, no label. Outlined shell,
// paperAlt fill. Intended for filter strips and compact pickers.
Rectangle {
    id: root
    property alias text: field.text
    property alias placeholderText: field.placeholderText
    property alias readOnly: field.readOnly
    property int fieldHeight: theme.controlHeightLg - 4

    signal editingFinished()
    signal accepted()

    implicitWidth: 240
    implicitHeight: fieldHeight
    color: Design.paperAlt(theme)
    border.color: field.activeFocus ? Design.ink(theme) : Design.line(theme)
    border.width: 1
    radius: theme.radiusSm

    Row {
        anchors.fill: parent
        anchors.leftMargin: theme.spacingSm + 2
        anchors.rightMargin: theme.spacingSm + 2
        spacing: theme.spacingSm

        Canvas {
            width: 14
            height: 14
            anchors.verticalCenter: parent.verticalCenter
            onPaint: {
                var ctx = getContext("2d")
                ctx.reset()
                ctx.strokeStyle = Design.ink3(theme)
                ctx.lineWidth = 1.2
                ctx.beginPath()
                ctx.arc(6, 6, 4, 0, Math.PI * 2)
                ctx.stroke()
                ctx.beginPath()
                ctx.moveTo(9, 9)
                ctx.lineTo(13, 13)
                ctx.stroke()
            }
        }

        TextField {
            id: field
            width: parent.width - 14 - parent.spacing - (parent.anchors.leftMargin + parent.anchors.rightMargin)
            height: parent.height
            color: Design.ink(theme)
            font.family: Design.sans()
            font.pixelSize: theme.fontBody
            placeholderTextColor: Design.ink3(theme)
            selectionColor: Design.accent(theme)
            selectedTextColor: Design.paper(theme)
            verticalAlignment: TextInput.AlignVCenter
            padding: 0
            background: Rectangle { color: "transparent"; border.width: 0 }

            onEditingFinished: root.editingFinished()
            onAccepted: root.accepted()
        }
    }
}
