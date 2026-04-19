import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Rectangle {
    id: root
    property int padding: 14
    property bool pad: true
    property color fillColor: Design.paperAlt(theme)
    property color strokeColor: Design.line(theme)
    property int cornerRadius: 2
    property string eyebrow: ""
    property string title: ""
    property string subtitle: ""
    property Component action: null
    property bool headerDividerVisible: headerVisible
    property int headerMinimumHeight: 36
    default property alias content: body.data

    readonly property bool headerVisible: eyebrow.length > 0 || title.length > 0 || subtitle.length > 0 || action !== null
    readonly property int bodyPadding: pad ? padding : 0

    color: fillColor
    radius: cornerRadius
    border.color: strokeColor
    border.width: 1
    clip: true
    implicitWidth: Math.max(240, headerVisible ? headerRow.implicitWidth + 28 : 240)
    implicitHeight: (headerVisible ? header.implicitHeight : 0) + body.implicitHeight + bodyPadding * 2

    Item {
        id: header
        visible: root.headerVisible
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: visible ? Math.max(root.headerMinimumHeight, headerRow.implicitHeight + 16) : 0

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            visible: root.headerDividerVisible
            height: 1
            color: root.strokeColor
        }

        Row {
            id: headerRow
            anchors.fill: parent
            anchors.leftMargin: 14
            anchors.rightMargin: 14
            anchors.topMargin: 8
            anchors.bottomMargin: 8
            spacing: 12

            Column {
                width: Math.max(0, parent.width - actionSlot.width - parent.spacing)
                spacing: subtitleText.visible ? 3 : 0

                Text {
                    id: eyebrowText
                    visible: root.eyebrow.length > 0
                    text: root.eyebrow
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 9
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
                    font.family: Design.serif(theme)
                    font.pixelSize: 14
                    font.weight: Font.Medium
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
                    font.pixelSize: 12
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

    Item {
        id: bodyFrame
        anchors.top: root.headerVisible ? header.bottom : parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom

        Item {
            id: body
            anchors.fill: parent
            anchors.margins: root.bodyPadding
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }
}
