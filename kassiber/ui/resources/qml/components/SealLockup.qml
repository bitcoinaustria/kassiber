import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property real size: 180
    property string ringText: "BITCOIN * ACCOUNTING * AUSTRIA * PRIVATE * LOCAL *"
    property color inkColor: Design.ink(theme)
    property color sealColor: Design.accent(theme)

    implicitWidth: size
    implicitHeight: size
    width: implicitWidth
    height: implicitHeight

    Rectangle {
        width: root.size * 0.88
        height: width
        radius: width / 2
        anchors.centerIn: parent
        color: "transparent"
        border.color: root.inkColor
        border.width: 1
    }

    Rectangle {
        width: root.size * 0.85
        height: width
        radius: width / 2
        anchors.centerIn: parent
        color: "transparent"
        border.color: root.inkColor
        border.width: 1
        opacity: 0.7
    }

    ArcText {
        anchors.fill: parent
        text: root.ringText
        radius: root.size * 0.36
        startAngle: -180
        sweepAngle: 360
        inkColor: root.inkColor
        fontFamily: Design.serif(theme)
        fontPixelSize: Math.max(8, Math.round(root.size * 0.058))
        letterSpacing: 0.8
    }

    Rectangle {
        width: root.size * 0.52
        height: width
        radius: width / 2
        anchors.centerIn: parent
        color: "transparent"
        border.color: root.inkColor
        border.width: 1
        opacity: 0.8
    }

    LogoMark {
        anchors.centerIn: parent
        size: root.size * 0.54
        strokeColor: root.inkColor
        sealColor: root.sealColor
    }
}
