import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

// Label + optional sub-label on the left, Toggle switch on the right.
Item {
    id: root
    property string label: ""
    property string description: ""
    property bool checked: false

    signal toggled()

    implicitWidth: 260
    implicitHeight: Math.max(switchBox.implicitHeight, textCol.implicitHeight)

    RowLayout {
        anchors.fill: parent
        spacing: theme.spacingSm + 4

        ColumnLayout {
            id: textCol
            Layout.fillWidth: true
            spacing: 2

            Text {
                Layout.fillWidth: true
                text: root.label
                color: Design.ink(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontBody
                wrapMode: Text.WordWrap
            }

            Text {
                visible: root.description.length > 0
                Layout.fillWidth: true
                text: root.description
                color: Design.ink3(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontBodySmall
                wrapMode: Text.WordWrap
            }
        }

        Button {
            id: switchBox
            flat: true
            padding: 0
            implicitWidth: 30
            implicitHeight: 16
            hoverEnabled: true
            onClicked: root.toggled()

            contentItem: Item {
                anchors.fill: parent

                Rectangle {
                    anchors.fill: parent
                    color: root.checked ? Design.ink(theme) : Design.line2(theme)
                }

                Rectangle {
                    width: 12
                    height: 12
                    y: 2
                    x: root.checked ? parent.width - 14 : 2
                    color: Design.paperAlt(theme)
                    Behavior on x { NumberAnimation { duration: 120 } }
                }
            }

            background: Rectangle { color: "transparent" }
        }
    }
}
