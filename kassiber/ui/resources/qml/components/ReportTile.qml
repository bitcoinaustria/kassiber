import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

Button {
    id: root
    property string title: ""
    property string subtitle: ""
    property string detail: ""
    property string iconGlyph: "\u2197"

    flat: true
    padding: 0
    hoverEnabled: true
    implicitHeight: 98

    contentItem: RowLayout {
        spacing: 14
        anchors.margins: 16
        anchors.fill: parent

        Rectangle {
            Layout.preferredWidth: 34
            Layout.preferredHeight: 34
            color: "transparent"
            border.color: Design.ink(theme)
            border.width: 1

            Text {
                anchors.centerIn: parent
                text: root.iconGlyph
                color: Design.ink(theme)
                font.family: Design.sans()
                font.pixelSize: 18
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            Text {
                text: root.title
                color: Design.ink(theme)
                font.family: Design.sans()
                font.pixelSize: 16
                font.weight: Font.DemiBold
            }

            Text {
                Layout.fillWidth: true
                text: root.subtitle
                color: Design.ink3(theme)
                font.family: Design.sans()
                font.pixelSize: 11
                elide: Text.ElideRight
            }

            Item { Layout.preferredHeight: 6 }

            Text {
                Layout.fillWidth: true
                text: root.detail
                color: Design.ink(theme)
                font.family: Design.mono(theme)
                font.pixelSize: 11
                elide: Text.ElideRight
            }
        }

        Text {
            text: "\u2197"
            color: Design.ink3(theme)
            font.family: Design.mono(theme)
            font.pixelSize: 12
            Layout.alignment: Qt.AlignTop
        }
    }

    background: Rectangle {
        color: root.hovered ? Design.paper(theme) : Design.paperAlt(theme)
        border.color: Design.line(theme)
        border.width: 1
        radius: 0
    }
}
