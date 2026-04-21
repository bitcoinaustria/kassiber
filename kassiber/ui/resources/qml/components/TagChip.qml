import QtQuick 2.15

import "Design.js" as Design

// Outlined mono tag chip used in the transactions table and the tx detail view.
Rectangle {
    id: root
    property string label: ""
    property int horizontalPadding: 7
    property int verticalPadding: 2

    implicitWidth: text.implicitWidth + horizontalPadding * 2
    implicitHeight: text.implicitHeight + verticalPadding * 2
    color: Design.paper(theme)
    border.color: Design.line(theme)
    border.width: 1
    radius: 0

    Text {
        id: text
        anchors.centerIn: parent
        text: root.label
        color: Design.ink2(theme)
        font.family: Design.mono(theme)
        font.pixelSize: theme.fontCaption
        font.letterSpacing: 0.4
    }
}
