import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property real size: 22
    property color inkColor: Design.ink(theme)
    property string text: "Kassiber"

    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight

    Row {
        id: row
        spacing: 9

        LogoMark {
            size: root.size * 1.15
            strokeColor: root.inkColor
            sealColor: Design.accent(theme)
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: root.text
            color: root.inkColor
            font.family: Design.serif(theme)
            font.pixelSize: root.size
            font.weight: Font.Medium
            font.letterSpacing: -0.2
        }
    }
}
