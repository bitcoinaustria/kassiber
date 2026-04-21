import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

// Single row in the Add Connection type picker: name (sans 14) + description
// (mono 10 upper ink3), trailing arrow.
Button {
    id: root
    property string connectionName: ""
    property string description: ""

    flat: true
    padding: 0
    hoverEnabled: true
    implicitHeight: 60

    contentItem: RowLayout {
        anchors.fill: parent
        anchors.leftMargin: theme.cardPadding
        anchors.rightMargin: theme.cardPadding
        spacing: theme.spacingSm + 4

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            Text {
                Layout.fillWidth: true
                text: root.connectionName
                color: Design.ink(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontHeadingXs
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }

            Text {
                Layout.fillWidth: true
                text: root.description.toUpperCase()
                color: Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontCaption
                font.letterSpacing: 0.4
                elide: Text.ElideRight
            }
        }

        Text {
            text: "\u2192"
            color: Design.ink3(theme)
            font.family: Design.mono(theme)
            font.pixelSize: theme.fontBodyStrong
        }
    }

    background: Rectangle {
        color: root.hovered ? Design.paper(theme) : "transparent"
        border.color: root.hovered ? Design.ink(theme) : Design.line(theme)
        border.width: 1
        radius: theme.radiusSm
    }
}
