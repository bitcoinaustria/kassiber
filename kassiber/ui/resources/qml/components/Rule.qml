import QtQuick 2.15

import "Design.js" as Design

Rectangle {
    id: root
    property bool vertical: false
    property color ruleColor: Design.line(theme)

    color: ruleColor
    implicitWidth: vertical ? 1 : 120
    implicitHeight: vertical ? 24 : 1
}
