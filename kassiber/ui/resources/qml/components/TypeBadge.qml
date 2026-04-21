import QtQuick 2.15

import "Design.js" as Design

Rectangle {
    id: root
    property string label: ""
    property string tone: "muted"
    property int horizontalPadding: 6
    property int verticalPadding: 2

    readonly property color toneColor: {
        if (tone === "Income" || tone === "positive") return theme.typeIncome
        if (tone === "Expense" || tone === "negative") return theme.typeExpense
        if (tone === "Transfer") return theme.typeTransfer
        if (tone === "Swap" || tone === "swap") return theme.typeSwap
        if (tone === "Consolidation") return theme.typeConsolidation
        if (tone === "Rebalance") return theme.typeRebalance
        if (tone === "Mint" || tone === "mint") return theme.typeMint
        if (tone === "Melt" || tone === "melt") return theme.typeMelt
        if (tone === "Fee") return theme.typeFee
        return Design.ink3(theme)
    }

    implicitWidth: labelText.implicitWidth + horizontalPadding * 2
    implicitHeight: labelText.implicitHeight + verticalPadding * 2
    color: "transparent"
    border.color: toneColor
    border.width: 1
    radius: 0

    Text {
        id: labelText
        anchors.centerIn: parent
        text: root.label
        color: root.toneColor
        font.family: Design.mono(theme)
        font.pixelSize: 9
        font.weight: Font.DemiBold
        font.capitalization: Font.AllUppercase
        font.letterSpacing: 1.0
    }
}
