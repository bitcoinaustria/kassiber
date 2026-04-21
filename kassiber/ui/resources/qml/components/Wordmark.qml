import QtQuick 2.15

import "Design.js" as Design

// Text-only wordmark used in AppHeader (no LogoMark for now).
Item {
    id: root
    property real size: 22
    property color inkColor: Design.ink(theme)
    property string text: "Kassiber"

    implicitWidth: label.implicitWidth
    implicitHeight: label.implicitHeight

    Text {
        id: label
        anchors.verticalCenter: parent.verticalCenter
        text: root.text
        color: root.inkColor
        font.family: Design.sans()
        font.pixelSize: root.size
        font.weight: Font.Medium
        font.letterSpacing: -0.2
    }
}
