import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

// Card — framed container with optional header + padded content slot.
//
// The card is self-sizing: its implicitHeight flows from its content's
// childrenRect, so parents that don't pin Layout.preferredHeight get a
// naturally-fit card. Content still fills width via `content.width = body.width`.
Rectangle {
    id: root

    default property alias content: bodyContent.data

    property string eyebrow: ""
    property string title: ""
    property string subtitle: ""
    property Component action: null

    property bool pad: true
    property int padding: theme.cardPadding
    property int cornerRadius: theme.radiusSm
    property color fillColor: Design.paperAlt(theme)
    property color strokeColor: Design.line(theme)

    readonly property bool headerVisible:
        eyebrow.length > 0 || title.length > 0 || subtitle.length > 0 || action !== null
    readonly property int innerPadding: pad ? padding : 0

    color: fillColor
    radius: cornerRadius
    border.color: strokeColor
    border.width: 1
    clip: true

    // implicit size flows from header + content
    implicitWidth: Math.max(
        240,
        (headerVisible ? headerRow.implicitWidth + 28 : 0),
        bodyContent.implicitWidth + innerPadding * 2
    )
    implicitHeight:
        (headerVisible ? header.height : 0)
        + bodyContent.implicitHeight
        + innerPadding * 2

    // Header ---------------------------------------------------------------
    Item {
        id: header
        visible: root.headerVisible
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: visible
            ? Math.max(theme.cardHeaderHeight, headerRow.implicitHeight + 16)
            : 0

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            visible: root.headerVisible
            height: 1
            color: root.strokeColor
        }

        Row {
            id: headerRow
            anchors.fill: parent
            anchors.leftMargin: theme.cardPadding
            anchors.rightMargin: theme.cardPadding
            anchors.topMargin: theme.spacingSm
            anchors.bottomMargin: theme.spacingSm
            spacing: theme.spacingSm + 4

            Column {
                width: Math.max(0, parent.width - actionSlot.width - parent.spacing)
                spacing: subtitleText.visible ? 3 : 0

                Text {
                    id: eyebrowText
                    visible: root.eyebrow.length > 0
                    text: root.eyebrow
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontMicro
                    font.weight: Font.DemiBold
                    font.capitalization: Font.AllUppercase
                    font.letterSpacing: 1.2
                }

                Text {
                    id: titleText
                    visible: root.title.length > 0
                    width: parent.width
                    text: root.title
                    color: Design.ink(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontBodyStrong
                    font.weight: Font.DemiBold
                    font.letterSpacing: 0.2
                    elide: Text.ElideRight
                }

                Text {
                    id: subtitleText
                    visible: root.subtitle.length > 0
                    width: parent.width
                    text: root.subtitle
                    color: Design.ink2(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontBody
                    wrapMode: Text.WordWrap
                }
            }

            Item {
                id: actionSlot
                width: actionLoader.active ? actionLoader.implicitWidth : 0
                height: actionLoader.active ? parent.height : 0

                Loader {
                    id: actionLoader
                    anchors.centerIn: parent
                    active: root.action !== null
                    sourceComponent: root.action
                }
            }
        }
    }

    // Body -----------------------------------------------------------------
    //
    // bodyContent is anchored top/left/right only — its height flows from
    // childrenRect. That lets root.implicitHeight pull a concrete value up
    // through the layout system. When a parent forces the card taller (e.g.
    // Layout.fillHeight so cards in a row share a height), the card frame
    // extends but bodyContent stays at content height; any extra vertical
    // space renders as the card fill color at the bottom.
    //
    // A content child that wants to occupy the full card area can position
    // itself relative to `root` (Card) rather than bodyContent.
    Item {
        id: bodyContent
        anchors.top: root.headerVisible ? header.bottom : parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: root.innerPadding
        anchors.rightMargin: root.innerPadding
        anchors.topMargin: root.innerPadding
        implicitHeight: childrenRect.height
        implicitWidth: childrenRect.width
    }
}
