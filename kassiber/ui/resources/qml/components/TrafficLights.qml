import QtQuick 2.15

Row {
    spacing: 8

    Repeater {
        model: ["#FF5F57", "#FEBC2E", "#28C840"]

        Rectangle {
            width: 12
            height: 12
            radius: 6
            color: modelData
            border.color: Qt.rgba(0, 0, 0, 0.15)
            border.width: 1
        }
    }
}
