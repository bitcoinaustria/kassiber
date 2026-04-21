import QtQuick 2.15

import "Design.js" as Design

// Small per-network badge used on backend rows in Settings.
// Palette matches settings.jsx NetworkBadge.
Rectangle {
    id: root
    property string network: "BTC"

    readonly property var networkColors: {
        "BTC":    { fg: "#b16a12", bg: "#F4E4D1", border: "#D0A470" },
        "LIQUID": { fg: "#3E5EA8", bg: "#DEE5F3", border: "#8FA1CF" },
        "LN":     { fg: "#7A3FA6", bg: "#E7DBF1", border: "#B192D0" },
        "FX":     { fg: Design.ink2(theme), bg: "transparent", border: Design.ink3(theme) }
    }

    readonly property var spec: networkColors[network] || networkColors.FX

    implicitWidth: label.implicitWidth + 16
    implicitHeight: label.implicitHeight + 4
    color: spec.bg
    border.color: spec.border
    border.width: 1

    Text {
        id: label
        anchors.centerIn: parent
        text: root.network
        color: root.spec.fg
        font.family: Design.mono(theme)
        font.pixelSize: theme.fontMicro
        font.weight: Font.Bold
        font.letterSpacing: 1.4
    }
}
