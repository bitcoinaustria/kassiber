import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Switch {
    id: control
    property string description: ""

    spacing: 12
    hoverEnabled: true
    implicitHeight: Math.max(indicator.implicitHeight, labelColumn.implicitHeight)

    indicator: Rectangle {
        implicitWidth: 36
        implicitHeight: 20
        x: 0
        y: parent.height / 2 - height / 2
        radius: height / 2
        color: control.checked ? Design.ink(theme) : Design.line2(theme)
        border.color: control.checked ? Design.ink(theme) : Design.line2(theme)
        border.width: 1
        opacity: control.enabled ? 1.0 : 0.45

        Rectangle {
            width: 16
            height: 16
            radius: 8
            x: control.checked ? parent.width - width - 2 : 2
            y: 2
            color: Design.paperAlt(theme)

            Behavior on x {
                NumberAnimation { duration: 120 }
            }
        }
    }

    contentItem: Column {
        id: labelColumn
        leftPadding: control.indicator.width + control.spacing
        spacing: control.description.length > 0 ? 2 : 0

        Text {
            visible: control.text.length > 0
            text: control.text
            color: Design.ink2(theme)
            font.family: Design.sans()
            font.pixelSize: 12
            font.weight: Font.Medium
        }

        Text {
            visible: control.description.length > 0
            text: control.description
            color: Design.ink3(theme)
            font.family: Design.sans()
            font.pixelSize: 11
            wrapMode: Text.WordWrap
        }
    }
}
