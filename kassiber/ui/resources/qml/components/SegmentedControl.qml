import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Item {
    id: root
    property var model: []
    property string currentId: ""
    property int itemHeight: 24
    property bool fillWidth: false
    property bool borderOuter: true
    property int fontPixelSize: 10

    signal activated(string id)

    implicitHeight: itemHeight
    implicitWidth: fillWidth ? 0 : row.implicitWidth

    Rectangle {
        visible: root.borderOuter
        anchors.fill: parent
        color: "transparent"
        border.color: Design.line(theme)
        border.width: 1
    }

    Row {
        id: row
        anchors.fill: parent
        spacing: root.fillWidth ? 0 : 0

        Repeater {
            model: root.model

            Button {
                id: btn
                padding: 0
                flat: true
                height: root.itemHeight
                width: root.fillWidth
                    ? (root.width / root.model.length)
                    : Math.max(28, label.implicitWidth + 16)
                hoverEnabled: true
                onClicked: {
                    root.currentId = modelData.id
                    root.activated(modelData.id)
                }

                contentItem: Text {
                    id: label
                    anchors.fill: parent
                    text: modelData.label
                    color: root.currentId === modelData.id
                        ? Design.paper(theme)
                        : Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: root.fontPixelSize
                    font.weight: Font.DemiBold
                    font.letterSpacing: 0.8
                    font.capitalization: Font.AllUppercase
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle {
                    color: root.currentId === modelData.id
                        ? Design.ink(theme)
                        : (btn.hovered ? Design.paperAlt(theme) : "transparent")
                    border.color: root.borderOuter
                        ? "transparent"
                        : (root.currentId === modelData.id
                            ? Design.ink(theme)
                            : Design.line(theme))
                    border.width: root.borderOuter ? 0 : 1
                }
            }
        }
    }
}
