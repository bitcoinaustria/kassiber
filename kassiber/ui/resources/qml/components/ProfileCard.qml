import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "Design.js" as Design

// Profile card used in the Profiles screen. Click to select.
Button {
    id: root
    property string profileName: ""
    property string role: "Owner"
    property string lastOpened: ""
    property string taxPolicy: ""
    property int accountsCount: 0
    property int walletsCount: 0
    property bool active: false

    flat: true
    padding: 0
    hoverEnabled: true
    implicitHeight: column.implicitHeight + theme.cardPadding * 2
    implicitWidth: 320

    contentItem: Item {
        anchors.fill: parent

        ColumnLayout {
            id: column
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.leftMargin: theme.cardPadding + 2
            anchors.rightMargin: theme.cardPadding + 2
            anchors.topMargin: theme.cardPadding
            spacing: theme.spacingSm + 2

            // Active chip (top-right absolute)
            RowLayout {
                Layout.fillWidth: true
                spacing: theme.spacingSm

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Text {
                        Layout.fillWidth: true
                        text: root.profileName
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontHeadingSm
                        font.weight: Font.DemiBold
                        font.letterSpacing: -0.2
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.role.toUpperCase() + "  \u00b7  OPENED " + root.lastOpened.toUpperCase()
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontMicro
                        font.letterSpacing: 1.0
                        elide: Text.ElideRight
                    }
                }

                Row {
                    visible: root.active
                    spacing: theme.spacingXs

                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 6
                        height: 6
                        radius: 3
                        color: Design.accent(theme)
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "ACTIVE"
                        color: Design.accent(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontMicro
                        font.weight: Font.Bold
                        font.letterSpacing: 1.4
                    }
                }
            }

            // Tax policy block (red left border)
            Item {
                Layout.fillWidth: true
                implicitHeight: taxCol.implicitHeight

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: 2
                    color: Design.accent(theme)
                }

                Column {
                    id: taxCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: theme.spacingSm + 2
                    spacing: 2

                    Text {
                        text: "TAX POLICY"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontMicro
                        font.letterSpacing: 1.2
                    }

                    Text {
                        width: parent.width
                        text: root.taxPolicy
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                        wrapMode: Text.WordWrap
                    }
                }
            }

            // Counts + open affordance
            RowLayout {
                Layout.fillWidth: true
                spacing: theme.spacingSm + 6

                Column {
                    spacing: 2

                    Text {
                        text: "ACCOUNTS"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontMicro
                        font.letterSpacing: 1.2
                    }

                    Text {
                        text: String(root.accountsCount)
                        color: Design.ink(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                    }
                }

                Column {
                    spacing: 2

                    Text {
                        text: "WALLETS"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontMicro
                        font.letterSpacing: 1.2
                    }

                    Text {
                        text: String(root.walletsCount)
                        color: Design.ink(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                    }
                }

                Item { Layout.fillWidth: true }

                Text {
                    Layout.alignment: Qt.AlignBottom
                    text: root.active ? "CURRENT \u2192" : "OPEN \u2192"
                    color: root.active ? Design.accent(theme) : Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontCaption
                    font.letterSpacing: 1.0
                }
            }
        }
    }

    background: Rectangle {
        color: root.active
            ? Design.paperAlt(theme)
            : (root.hovered ? Design.paperAlt(theme) : Design.paper(theme))
        border.color: root.active ? Design.ink(theme) : Design.line(theme)
        border.width: 1
        radius: theme.radiusSm
    }
}
