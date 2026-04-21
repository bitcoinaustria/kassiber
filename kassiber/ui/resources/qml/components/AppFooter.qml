import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

Item {
    id: root
    property string versionText: "v0.1.0"
    property string priceText: "\u20ac 71,420.18"
    property string priceSinceLabel: "just now"
    property string networkLabel: "MAINNET"
    property string donateLabel: "DONATE SATS"
    property string rateSourceLabel: "COINGECKO"
    property bool refreshing: false

    signal refreshClicked()
    signal donateClicked()
    signal githubClicked()

    implicitHeight: 28
    height: 28

    Rectangle {
        anchors.fill: parent
        color: Design.paper(theme)

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 1
            color: Design.line(theme)
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 18
            anchors.rightMargin: 18
            spacing: 18

            Text {
                text: "KASSIBER " + root.versionText
                color: Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: 10
                font.letterSpacing: 0.5
            }

            Row {
                spacing: 6

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 6
                    height: 6
                    radius: 3
                    color: theme.positive
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "WATCH-ONLY \u00b7 LOCAL ENCRYPTED VAULT"
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 10
                    font.letterSpacing: 0.5
                }
            }

            Item { Layout.fillWidth: true }

            Button {
                flat: true
                padding: 0
                hoverEnabled: true
                implicitWidth: donateText.implicitWidth + 28
                implicitHeight: 28
                onClicked: root.donateClicked()

                contentItem: Row {
                    anchors.centerIn: parent
                    spacing: 6

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "\u2665"
                        color: Design.accent(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 11
                    }

                    Text {
                        id: donateText
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.donateLabel
                        color: Design.accent(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
                        font.letterSpacing: 1.0
                    }
                }

                background: Rectangle {
                    color: Design.paperAlt(theme)
                    border.color: Design.line(theme)
                    border.width: 1
                }
            }

            Item { Layout.fillWidth: true }

            Row {
                spacing: 6
                Layout.alignment: Qt.AlignVCenter

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "BTC/EUR"
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 10
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.priceText
                    color: Design.ink(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 10
                    font.weight: Font.DemiBold
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "\u00b7 " + root.rateSourceLabel
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 10
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    flat: true
                    padding: 0
                    implicitWidth: 16
                    implicitHeight: 16
                    onClicked: root.refreshClicked()

                    contentItem: Text {
                        anchors.fill: parent
                        text: "\u21bb"
                        color: Design.ink2(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
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

            Text {
                text: root.networkLabel
                color: Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: 10
                font.letterSpacing: 0.5
            }

            Button {
                flat: true
                padding: 0
                implicitHeight: 16
                onClicked: root.githubClicked()

                contentItem: Text {
                    text: "GITHUB"
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: 10
                    font.letterSpacing: 0.5
                }

                background: Rectangle { color: "transparent" }
            }
        }
    }
}
