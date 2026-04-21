import QtQuick 2.15
import QtQuick.Controls 2.15
import Qt5Compat.GraphicalEffects

import "Design.js" as Design

Button {
    id: root
    property string connectionLabel: ""
    property string balanceLabel: ""
    property string statusTone: "ok"
    property bool borderTop: true
    property bool hideSensitive: false

    flat: true
    padding: 0
    hoverEnabled: true
    implicitHeight: 34

    contentItem: Item {
        anchors.fill: parent

        Rectangle {
            visible: root.borderTop
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 1
            color: Design.line(theme)
        }

        Text {
            id: label
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            width: parent.width - balance.width - dot.width - 28
            text: root.connectionLabel
            color: Design.ink(theme)
            font.family: Design.sans()
            font.pixelSize: 12
            font.weight: Font.Medium
            elide: Text.ElideRight
        }

        Text {
            id: balance
            anchors.right: dot.left
            anchors.verticalCenter: parent.verticalCenter
            anchors.rightMargin: 6
            text: root.balanceLabel
            color: Design.ink(theme)
            font.family: Design.mono(theme)
            font.pixelSize: 11
            layer.enabled: root.hideSensitive
            layer.effect: FastBlur { radius: 48 }
        }

        StatusDot {
            id: dot
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.rightMargin: 12
            tone: root.statusTone
        }
    }

    background: Rectangle {
        color: root.hovered ? Design.paper(theme) : "transparent"
    }
}
