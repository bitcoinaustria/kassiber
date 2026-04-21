import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property string text: ""
    property real radius: 72
    property real startAngle: -180
    property real sweepAngle: 360
    property real centerX: width / 2
    property real centerY: height / 2
    property real tangentOffset: 90
    property color inkColor: Design.ink(theme)
    property string fontFamily: Design.serif(theme)
    property int fontPixelSize: 10
    property real letterSpacing: 3

    Repeater {
        model: root.text.length

        delegate: Text {
            readonly property real angle: root.text.length > 1 ? root.startAngle + (root.sweepAngle * index) / (root.text.length - 1) : root.startAngle
            readonly property real radians: angle * Math.PI / 180.0
            readonly property real px: root.centerX + Math.cos(radians) * root.radius
            readonly property real py: root.centerY + Math.sin(radians) * root.radius

            text: root.text.charAt(index)
            visible: text.length > 0
            color: root.inkColor
            font.family: root.fontFamily
            font.pixelSize: root.fontPixelSize
            font.letterSpacing: root.letterSpacing
            transformOrigin: Item.Center
            x: px - width / 2
            y: py - height / 2
            rotation: angle + root.tangentOffset
        }
    }
}
