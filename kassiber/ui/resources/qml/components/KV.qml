import QtQuick 2.15

import "Design.js" as Design

Column {
    id: root
    property string label: ""
    property string value: ""
    property bool mono: true
    property color valueColor: Design.ink(theme)
    property int labelFontSize: theme.fontCaption
    property int valueFontSize: mono ? theme.fontBodyStrong : theme.fontHeadingXs

    spacing: 3

    Text {
        visible: root.label.length > 0
        text: root.label
        color: Design.ink3(theme)
        font.family: Design.sans()
        font.pixelSize: root.labelFontSize
        font.weight: Font.DemiBold
        font.capitalization: Font.AllUppercase
        font.letterSpacing: 1.2
    }

    Text {
        text: root.value
        color: root.valueColor
        font.family: root.mono ? Design.mono(theme) : Design.sans()
        font.pixelSize: root.valueFontSize
        elide: Text.ElideRight
        textFormat: Text.StyledText
    }
}
