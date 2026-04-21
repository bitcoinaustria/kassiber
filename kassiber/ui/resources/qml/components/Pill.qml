import QtQuick 2.15
import QtQuick.Controls 2.15

import "Design.js" as Design

Button {
    id: control
    property bool active: false
    property bool interactive: true
    property string tone: "ink"
    property bool mono: true

    readonly property var toneSpec: Design.pillTone(theme, tone)

    padding: 0
    leftPadding: 12
    rightPadding: 12
    topPadding: 0
    bottomPadding: 0
    implicitHeight: 26
    implicitWidth: contentText.implicitWidth + leftPadding + rightPadding
    enabled: control.interactive
    hoverEnabled: control.interactive

    contentItem: Text {
        id: contentText
        text: control.text
        color: control.active ? Design.paper(theme) : control.toneSpec.fg
        font.family: control.mono ? Design.mono(theme) : Design.sans()
        font.pixelSize: 11
        font.weight: Font.Medium
        font.letterSpacing: 0.4
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    background: Rectangle {
        color: control.active
            ? control.toneSpec.fg
            : ((control.interactive && control.hovered) ? Design.paperAlt(theme) : "transparent")
        radius: height / 2
        border.color: control.toneSpec.border
        border.width: 1
        opacity: control.interactive ? 1.0 : 0.8

        Behavior on color {
            ColorAnimation { duration: 120 }
        }
    }
}
