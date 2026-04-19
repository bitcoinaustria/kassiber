import QtQuick 2.15

import "Design.js" as Design

Item {
    id: root
    property string title: "Kassiber"
    property int titleBarHeight: 32
    property int cornerRadius: 10
    default property alias content: body.data

    implicitWidth: Math.max(480, body.implicitWidth)
    implicitHeight: titleBarHeight + body.implicitHeight

    Rectangle {
        anchors.fill: frame
        anchors.margins: -6
        anchors.topMargin: 4
        anchors.bottomMargin: -16
        y: 12
        radius: root.cornerRadius + 6
        color: Design.ink(theme)
        opacity: 0.09
        z: -2
    }

    Rectangle {
        anchors.fill: frame
        anchors.margins: -2
        anchors.bottomMargin: -8
        y: 6
        radius: root.cornerRadius + 2
        color: Design.ink(theme)
        opacity: 0.08
        z: -1
    }

    Rectangle {
        id: frame
        anchors.fill: parent
        radius: root.cornerRadius
        clip: true
        color: Design.paper(theme)
        border.color: Qt.rgba(0, 0, 0, 0.22)
        border.width: 1

        Rectangle {
            id: titleBar
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            height: root.titleBarHeight
            color: Design.ink(theme)

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Qt.rgba(0, 0, 0, 0.4)
            }

            TrafficLights {
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
            }

            Text {
                anchors.centerIn: parent
                text: root.title
                color: Qt.rgba(1, 1, 1, 0.55)
                font.family: Design.sans()
                font.pixelSize: 12
                font.weight: Font.Medium
                font.letterSpacing: 0.2
            }
        }

        Item {
            id: body
            anchors.top: titleBar.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }
}
