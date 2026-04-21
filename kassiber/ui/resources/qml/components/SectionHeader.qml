import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property string label: ""
    property string title: ""
    property string subtitle: ""
    property int titleSize: 28
    property Component action: null

    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight

    Row {
        id: row
        spacing: 16

        Column {
            width: actionLoader.active ? Math.max(0, root.width - actionLoader.implicitWidth - row.spacing) : implicitWidth
            spacing: subtitleText.visible ? 6 : 2

            Text {
                visible: root.label.length > 0
                text: root.label
                color: Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: 10
                font.weight: Font.DemiBold
                font.capitalization: Font.AllUppercase
                font.letterSpacing: 1.4
            }

            Text {
                width: parent.width
                visible: root.title.length > 0
                text: root.title
                color: Design.ink(theme)
                font.family: Design.serif(theme)
                font.pixelSize: root.titleSize
                font.weight: Font.Normal
                font.letterSpacing: -0.2
                wrapMode: Text.WordWrap
            }

            Text {
                id: subtitleText
                width: parent.width
                visible: root.subtitle.length > 0
                text: root.subtitle
                color: Design.ink2(theme)
                font.family: Design.sans()
                font.pixelSize: 13
                wrapMode: Text.WordWrap
            }
        }

        Item {
            width: Math.max(actionLoader.implicitWidth, 0)
            height: Math.max(actionLoader.implicitHeight, 0)

            Loader {
                id: actionLoader
                anchors.centerIn: parent
                active: root.action !== null
                sourceComponent: root.action
            }
        }
    }
}
