import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

// Big A/B selection card used in the Welcome encrypt step.
// Letter prefix + title + tagline + description. Accent border when `warning`.
Button {
    id: root
    property string letter: "A"
    property string title: ""
    property string tagline: ""
    property string description: ""
    property bool active: false
    property bool warning: false

    flat: true
    padding: 0
    hoverEnabled: true
    implicitHeight: column.implicitHeight + theme.cardPadding * 2

    contentItem: Item { anchors.fill: parent }

    background: Rectangle {
        color: root.active ? Design.paperAlt(theme) : Design.paper(theme)
        border.color: root.active
            ? (root.warning ? Design.accent(theme) : Design.ink(theme))
            : Design.line(theme)
        border.width: 1
        radius: theme.radiusSm

        // Warning chip (top-right)
        Rectangle {
            visible: root.warning
            anchors.top: parent.top
            anchors.right: parent.right
            width: warningText.implicitWidth + 16
            height: warningText.implicitHeight + 6
            color: Design.accent(theme)

            Text {
                id: warningText
                anchors.centerIn: parent
                text: "\u26a0 INSECURE"
                color: Design.paper(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontMicro
                font.weight: Font.Bold
                font.letterSpacing: 1.6
            }
        }

        Column {
            id: column
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.leftMargin: theme.cardPadding + 2
            anchors.rightMargin: theme.cardPadding + 2
            anchors.topMargin: theme.cardPadding
            spacing: theme.spacingSm - 2

            Row {
                width: parent.width
                spacing: theme.spacingSm + 2

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.letter
                    color: root.active ? Design.accent(theme) : Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontBodySmall
                    font.weight: Font.Bold
                    font.letterSpacing: 1.2
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.title
                    color: Design.ink(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontHeadingLg
                    font.weight: Font.DemiBold
                    font.letterSpacing: -0.2
                }

                Item { width: 10; height: 1 }

                Text {
                    visible: root.active && !root.warning
                    anchors.verticalCenter: parent.verticalCenter
                    text: "\u25cf SELECTED"
                    color: Design.accent(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontMicro
                    font.weight: Font.Bold
                    font.letterSpacing: 1.4
                }
            }

            Text {
                width: parent.width
                text: root.tagline.toUpperCase()
                color: root.warning ? Design.accent(theme) : Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontMicro
                font.letterSpacing: 1.4
                font.weight: root.warning ? Font.Bold : Font.Normal
            }

            Text {
                width: parent.width
                text: root.description
                color: Design.ink2(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontBody + 0.5
                wrapMode: Text.WordWrap
                lineHeight: 1.4
            }
        }
    }
}
