import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

// Large selectable export tile: headline format name + sub-label + detail.
// When primary=true uses the accent ink fill (filled dark) to stand out.
Button {
    id: root
    property string formatName: "CSV"
    property string subtitle: ""
    property string detail: ""
    property bool primary: false
    property bool interactive: false

    flat: true
    padding: 0
    hoverEnabled: root.interactive
    implicitHeight: column.implicitHeight + theme.cardPadding * 2

    contentItem: Item {
        anchors.fill: parent
    }

    background: Rectangle {
        color: root.primary
            ? Design.ink(theme)
            : ((root.interactive && root.hovered) ? Design.paper(theme) : Design.paperAlt(theme))
        border.color: root.primary ? Design.ink(theme) : Design.line(theme)
        border.width: 1
        radius: theme.radiusSm

        Column {
            id: column
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.leftMargin: theme.cardPadding
            anchors.rightMargin: theme.cardPadding
            anchors.topMargin: theme.cardPadding
            spacing: 2

            Row {
                width: parent.width
                spacing: theme.spacingSm

                Text {
                    width: parent.width - (arrow.visible ? arrow.implicitWidth + parent.spacing : 0)
                    text: root.formatName
                    color: root.primary ? Design.paper(theme) : Design.ink(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontHeadingLg
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }

                Text {
                    id: arrow
                    visible: root.interactive
                    text: "\u2913"
                    color: root.primary ? Design.paper(theme) : Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontHeadingXs
                }
            }

            Text {
                visible: root.subtitle.length > 0
                width: parent.width
                text: root.subtitle
                color: root.primary ? Design.paper(theme) : Design.ink2(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontBodySmall
                opacity: root.primary ? 0.8 : 1.0
            }

            Item { width: 1; height: theme.spacingSm - 2 }

            Text {
                visible: root.detail.length > 0
                width: parent.width
                text: root.detail
                color: root.primary ? Design.paper(theme) : Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontCaption
                opacity: root.primary ? 0.7 : 1.0
            }
        }
    }
}
