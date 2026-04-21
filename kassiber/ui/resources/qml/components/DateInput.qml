import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Item {
    id: root
    property string label: ""
    property string value: ""
    property string placeholderText: "yyyy-mm-dd"

    signal activated()

    implicitWidth: 180
    implicitHeight: column.implicitHeight

    Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: 6

        Text {
            visible: root.label.length > 0
            text: root.label
            color: Design.ink2(theme)
            font.family: Design.sans()
            font.pixelSize: 10
            font.weight: Font.DemiBold
            font.letterSpacing: 1.4
            font.capitalization: Font.AllUppercase
        }

        Button {
            id: field
            width: parent.width
            height: 36
            padding: 0
            flat: true
            hoverEnabled: true
            onClicked: root.activated()

            contentItem: Item {
                anchors.fill: parent

                Text {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: 10
                    text: root.value.length > 0 ? root.value : root.placeholderText
                    color: root.value.length > 0
                        ? Design.ink(theme)
                        : Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 12
                }

                Canvas {
                    width: 14
                    height: 14
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.rightMargin: 10
                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.reset()
                        ctx.strokeStyle = Design.ink2(theme)
                        ctx.lineWidth = 1
                        ctx.strokeRect(1.5, 3.5, 11, 9)
                        ctx.beginPath()
                        ctx.moveTo(1.5, 6.5)
                        ctx.lineTo(12.5, 6.5)
                        ctx.stroke()
                        ctx.beginPath()
                        ctx.moveTo(4.5, 2); ctx.lineTo(4.5, 4.5)
                        ctx.moveTo(9.5, 2); ctx.lineTo(9.5, 4.5)
                        ctx.stroke()
                    }
                }
            }

            background: Rectangle {
                color: Design.paperAlt(theme)
                border.color: field.hovered
                    ? Design.ink(theme)
                    : Design.line(theme)
                border.width: 1
                radius: 2
            }
        }
    }
}
