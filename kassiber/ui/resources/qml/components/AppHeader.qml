import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

Item {
    id: root
    property var navModel: [
        { id: "overview", label: "Overview" },
        { id: "transactions", label: "Transactions" },
        { id: "reports", label: "Reports" }
    ]
    property string currentRoute: "overview"
    property string workspaceLabel: "My Books"
    property string profileLabel: "Alice"
    property string currentLang: "EN"
    property bool hideSensitive: false

    signal routeSelected(string id)
    signal workspaceClicked()
    signal langSelected(string code)
    signal hideSensitiveToggled()
    signal lockClicked()
    signal settingsClicked()

    implicitHeight: 54
    height: 54

    Rectangle {
        anchors.fill: parent
        color: Design.paper(theme)

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Design.ink(theme)
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 18
            anchors.rightMargin: 18
            spacing: 18

            Wordmark {
                size: 20
                inkColor: Design.ink(theme)
            }

            Rectangle {
                width: 1
                Layout.topMargin: 16
                Layout.bottomMargin: 16
                Layout.fillHeight: true
                color: Design.line(theme)
            }

            Row {
                spacing: 4

                Repeater {
                    model: root.navModel

                    Button {
                        id: navBtn
                        flat: true
                        padding: 0
                        hoverEnabled: true
                        width: navLabel.implicitWidth + 24
                        height: 30
                        onClicked: root.routeSelected(modelData.id)

                        contentItem: Item {
                            Text {
                                id: navLabel
                                anchors.centerIn: parent
                                text: modelData.label
                                color: root.currentRoute === modelData.id
                                    ? Design.ink(theme)
                                    : Design.ink3(theme)
                                font.family: Design.sans()
                                font.pixelSize: 12
                                font.weight: Font.Medium
                                font.letterSpacing: 0.2
                            }

                            Rectangle {
                                visible: root.currentRoute === modelData.id
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                anchors.leftMargin: 10
                                anchors.rightMargin: 10
                                height: 2
                                color: Design.accent(theme)
                            }
                        }

                        background: Rectangle { color: "transparent" }
                    }
                }
            }

            Item { Layout.fillWidth: true }

            Button {
                id: workspaceBtn
                flat: true
                padding: 0
                hoverEnabled: true
                implicitHeight: 26
                onClicked: root.workspaceClicked()

                contentItem: Row {
                    anchors.fill: parent
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    spacing: 8

                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 10
                        height: 10
                        color: "transparent"
                        border.color: Design.ink2(theme)
                        border.width: 1
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.workspaceLabel
                        color: Design.ink2(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 11
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "/ " + root.profileLabel
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
                    }
                }

                background: Rectangle {
                    color: workspaceBtn.hovered ? Design.paper(theme) : "transparent"
                    border.color: Design.line(theme)
                    border.width: 1
                }
            }

            Row {
                spacing: 0

                Repeater {
                    model: ["EN", "DE"]

                    Button {
                        id: langBtn
                        flat: true
                        padding: 0
                        hoverEnabled: true
                        implicitWidth: 28
                        implicitHeight: 24
                        onClicked: root.langSelected(modelData)

                        contentItem: Text {
                            anchors.fill: parent
                            text: modelData
                            color: root.currentLang === modelData
                                ? Design.paper(theme)
                                : Design.ink2(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.letterSpacing: 0.8
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        background: Rectangle {
                            color: root.currentLang === modelData
                                ? Design.ink(theme)
                                : "transparent"
                            border.color: Design.line(theme)
                            border.width: 1
                        }
                    }
                }
            }

            Button {
                flat: true
                padding: 0
                hoverEnabled: true
                implicitWidth: 26
                implicitHeight: 26
                onClicked: root.hideSensitiveToggled()

                contentItem: Text {
                    anchors.fill: parent
                    text: root.hideSensitive ? "\u25c9" : "\u25ce"
                    color: root.hideSensitive ? Design.paper(theme) : Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 12
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle {
                    color: root.hideSensitive ? Design.ink(theme) : "transparent"
                    border.color: root.hideSensitive ? Design.ink(theme) : Design.line(theme)
                    border.width: 1
                }
            }

            Button {
                flat: true
                padding: 0
                hoverEnabled: true
                implicitWidth: 26
                implicitHeight: 26
                onClicked: root.lockClicked()

                contentItem: Text {
                    anchors.fill: parent
                    text: "\ud83d\udd12"
                    color: Design.ink2(theme)
                    font.pixelSize: 11
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle {
                    color: "transparent"
                    border.color: Design.line(theme)
                    border.width: 1
                }
            }

            Button {
                flat: true
                padding: 0
                hoverEnabled: true
                implicitWidth: 26
                implicitHeight: 26
                onClicked: root.settingsClicked()

                contentItem: Text {
                    anchors.fill: parent
                    text: "\u2699"
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 13
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle {
                    color: "transparent"
                    border.color: Design.line(theme)
                    border.width: 1
                }
            }
        }
    }
}
