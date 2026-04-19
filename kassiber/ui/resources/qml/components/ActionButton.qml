import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Button {
    id: control
    property string variant: "primary"
    property string size: "md"
    property int minWidth: 0
    property Component leading: null
    property Component trailing: null
    property bool mono: false
    property int cornerRadius: 2

    readonly property var paletteSpec: Design.buttonColors(theme, variant, enabled, down, hovered)
    readonly property int horizontalPaddingAmount: Design.paddingFor(size)
    readonly property int fontSize: Design.fontSizeFor(size)

    padding: 0
    leftPadding: horizontalPaddingAmount
    rightPadding: horizontalPaddingAmount
    topPadding: 0
    bottomPadding: 0
    spacing: 8
    hoverEnabled: true
    implicitHeight: Design.heightFor(size)
    implicitWidth: Math.max(minWidth, contentShell.implicitWidth + leftPadding + rightPadding)

    contentItem: Item {
        id: contentShell
        implicitWidth: row.implicitWidth
        implicitHeight: row.implicitHeight

        Row {
            id: row
            anchors.centerIn: parent
            spacing: control.spacing

            Loader {
                active: control.leading !== null
                visible: active
                sourceComponent: control.leading
            }

            Text {
                text: control.text
                color: control.paletteSpec.fg
                font.family: control.mono ? Design.mono(theme) : Design.sans()
                font.pixelSize: control.fontSize
                font.weight: Font.Medium
                font.letterSpacing: 0.2
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
            }

            Loader {
                active: control.trailing !== null
                visible: active
                sourceComponent: control.trailing
            }
        }
    }

    background: Rectangle {
        color: control.paletteSpec.bg
        radius: control.cornerRadius
        border.color: control.paletteSpec.border
        border.width: 1
        opacity: control.paletteSpec.opacity

        Behavior on color {
            ColorAnimation { duration: 120 }
        }

        Behavior on opacity {
            NumberAnimation { duration: 120 }
        }
    }
}
