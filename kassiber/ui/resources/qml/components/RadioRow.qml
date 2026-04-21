import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

// Single radio option, framed box with label + description. Drives a parent
// property via the `selected` binding + `activated` signal.
Rectangle {
    id: root
    property bool selected: false
    property bool interactive: false
    property string label: ""
    property string description: ""

    signal activated()

    implicitWidth: Math.max(180, labelText.implicitWidth + 64)
    implicitHeight: column.implicitHeight + 20
    color: selected ? Design.paper(theme) : "transparent"
    border.color: selected ? Design.ink(theme) : Design.line(theme)
    border.width: 1
    radius: theme.radiusSm

    MouseArea {
        anchors.fill: parent
        visible: root.interactive
        enabled: root.interactive
        cursorShape: Qt.PointingHandCursor
        onClicked: root.activated()
    }

    Row {
        anchors.fill: parent
        anchors.leftMargin: theme.spacingSm + 2
        anchors.rightMargin: theme.spacingSm + 2
        anchors.topMargin: 10
        anchors.bottomMargin: 10
        spacing: theme.spacingSm + 2

        // Radio dot
        Rectangle {
            width: 14
            height: 14
            radius: 7
            anchors.verticalCenter: parent.verticalCenter
            color: "transparent"
            border.color: root.selected ? Design.accent(theme) : Design.line2(theme)
            border.width: 1

            Rectangle {
                visible: root.selected
                anchors.centerIn: parent
                width: 8
                height: 8
                radius: 4
                color: Design.accent(theme)
            }
        }

        Column {
            id: column
            width: parent.width - 14 - parent.spacing
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2

            Text {
                id: labelText
                text: root.label
                color: Design.ink(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontBody
                font.weight: Font.DemiBold
            }

            Text {
                visible: root.description.length > 0
                width: parent.width
                text: root.description
                color: Design.ink3(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontBodySmall
                wrapMode: Text.WordWrap
            }
        }
    }
}
