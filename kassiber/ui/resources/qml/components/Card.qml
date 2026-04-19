import QtQuick 2.15

Rectangle {
    id: root
    property int padding: theme.spacingLg
    property color fillColor: theme.card
    property color strokeColor: theme.cardBorder
    default property alias content: body.data

    color: fillColor
    radius: theme.radiusLg
    border.color: strokeColor
    border.width: 1
    implicitWidth: body.implicitWidth + padding * 2
    implicitHeight: body.implicitHeight + padding * 2

    Item {
        id: body
        anchors.fill: parent
        anchors.margins: root.padding
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
    }
}
