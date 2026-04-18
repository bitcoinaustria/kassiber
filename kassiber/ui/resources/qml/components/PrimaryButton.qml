import QtQuick 2.15
import QtQuick.Controls 2.15

Button {
    id: control
    implicitHeight: 44
    implicitWidth: Math.max(180, contentItem.implicitWidth + 28)

    contentItem: Text {
        text: control.text
        color: "#FFFFFF"
        font.family: theme.bodyFont
        font.pixelSize: 14
        font.bold: true
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    background: Rectangle {
        color: control.enabled ? theme.accent : theme.accentDim
        radius: theme.radiusMd
    }
}
