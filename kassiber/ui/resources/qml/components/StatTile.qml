import QtQuick 2.15
import QtQuick.Layouts 1.15
import Qt5Compat.GraphicalEffects

import "Design.js" as Design

// Small boxed stat tile: eyebrow label + big mono value + optional sub-label.
// Self-sizing height. Used on Connection Detail and Tax preview.
Rectangle {
    id: root
    property string label: ""
    property string value: ""
    property string sub: ""
    property color valueColor: Design.ink(theme)
    property int paddingAmount: theme.cardPadding
    property bool blurred: false

    implicitWidth: Math.max(160, column.implicitWidth + paddingAmount * 2)
    implicitHeight: column.implicitHeight + paddingAmount * 2

    color: Design.paperAlt(theme)
    border.color: Design.line(theme)
    border.width: 1
    radius: theme.radiusSm

    Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.leftMargin: root.paddingAmount
        anchors.rightMargin: root.paddingAmount
        anchors.topMargin: root.paddingAmount
        spacing: 6

        Text {
            text: root.label
            color: Design.ink3(theme)
            font.family: Design.sans()
            font.pixelSize: theme.fontCaption
            font.weight: Font.DemiBold
            font.capitalization: Font.AllUppercase
            font.letterSpacing: 1.2
        }

        Text {
            width: parent.width
            text: root.value
            color: root.valueColor
            font.family: Design.mono(theme)
            font.pixelSize: theme.fontHeadingMd
            font.letterSpacing: -0.2
            elide: Text.ElideRight
            textFormat: Text.PlainText
            layer.enabled: root.blurred
            layer.effect: FastBlur { radius: 56 }
        }

        Text {
            visible: root.sub.length > 0
            width: parent.width
            text: root.sub
            color: Design.ink3(theme)
            font.family: Design.mono(theme)
            font.pixelSize: theme.fontCaption
            font.letterSpacing: 0.5
            wrapMode: Text.WordWrap
        }
    }
}
