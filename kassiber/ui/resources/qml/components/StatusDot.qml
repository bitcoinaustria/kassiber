import QtQuick 2.15

import "Design.js" as Design

Rectangle {
    id: root
    property string tone: "ok"
    property int dotSize: theme.dotSize

    implicitWidth: dotSize
    implicitHeight: dotSize
    width: dotSize
    height: dotSize
    radius: dotSize / 2

    color: {
        if (tone === "ok" || tone === "positive") return theme.positive
        if (tone === "warn" || tone === "syncing" || tone === "accent") return Design.accent(theme)
        if (tone === "err") return Design.err(theme)
        return Design.ink3(theme)
    }
}
