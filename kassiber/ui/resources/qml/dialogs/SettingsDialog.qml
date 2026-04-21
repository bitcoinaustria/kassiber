import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Static Settings modal. Reuses ToggleRow / ActionButton / NetworkBadge / Pill.
// Sections: Privacy, App lock, Data, Sync backends, Danger.
Dialog {
    id: root
    title: "Settings"
    modal: true
    width: 600
    padding: 0
    standardButtons: Dialog.NoButton
    anchors.centerIn: parent
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    property var mockBackends: [
        { name: "mempool.space",      url: "https://mempool.space/api",            net: "BTC",    health: "#893,014 \u00b7 2m", on: true  },
        { name: "local electrs",      url: "tcp://127.0.0.1:50001",                net: "BTC",    health: "\u2014",              on: false },
        { name: "Blockstream Liquid", url: "https://blockstream.info/liquid/api",  net: "LIQUID", health: "\u2014",              on: false },
        { name: "CoinGecko",          url: "https://api.coingecko.com/api/v3",     net: "FX",     health: "\u20ac71,420 \u00b7 14s", on: true }
    ]

    property var mockIdleOptions: [1, 5, 15, 30, 60]
    property int activeIdle: 5

    background: Rectangle {
        color: Design.paper(theme)
        border.color: Design.ink(theme)
        border.width: 1
    }

    header: Rectangle {
        height: 44
        color: Design.paper(theme)

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Design.line(theme)
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: theme.cardPadding
            anchors.rightMargin: theme.cardPadding

            Text {
                text: root.title
                color: Design.ink(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontHeadingMd
                font.weight: Font.DemiBold
            }

            Item { Layout.fillWidth: true }

            Button {
                flat: true
                padding: 0
                implicitWidth: 24
                implicitHeight: 24
                onClicked: root.close()

                contentItem: Text {
                    anchors.fill: parent
                    text: "\u2715"
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontCaption
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle { color: "transparent" }
            }
        }
    }

    contentItem: ScrollView {
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.availableWidth
            spacing: theme.gridGap + 4

            // Section helper component ---------------------------------------
            Component {
                id: sectionHeader
                Column {
                    width: parent ? parent.width : 0
                    spacing: 6

                    property string title: ""

                    Text {
                        text: title
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.weight: Font.Bold
                        font.letterSpacing: 1.4
                        font.capitalization: Font.AllUppercase
                    }

                    Rectangle {
                        width: parent.width
                        height: 1
                        color: Design.line(theme)
                    }
                }
            }

            // ---------------- Privacy ---------------------------------------
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 4
                Layout.rightMargin: theme.cardPadding + 4
                Layout.topMargin: theme.cardPadding + 4
                spacing: theme.spacingSm + 2

                Loader {
                    Layout.fillWidth: true
                    sourceComponent: sectionHeader
                    onLoaded: item.title = "Privacy"
                }

                ToggleRow {
                    Layout.fillWidth: true
                    label: "Hide sensitive data"
                    description: "Blur balances, addresses, and amounts throughout the UI."
                    checked: true
                }

                ToggleRow {
                    Layout.fillWidth: true
                    label: "Clear clipboard after 30s"
                    description: "Auto-clear copied addresses and keys."
                    checked: true
                }
            }

            // ---------------- App lock -------------------------------------
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 4
                Layout.rightMargin: theme.cardPadding + 4
                spacing: theme.spacingSm + 2

                Loader {
                    Layout.fillWidth: true
                    sourceComponent: sectionHeader
                    onLoaded: item.title = "App lock"
                }

                ToggleRow {
                    Layout.fillWidth: true
                    label: "Auto-lock when idle"
                    description: "Require passphrase to re-enter after a period of inactivity."
                    checked: true
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.spacingSm

                    Text {
                        text: "Idle timeout"
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                    }

                    Item { Layout.fillWidth: true }

                    Row {
                        spacing: theme.spacingXs

                        Repeater {
                            model: root.mockIdleOptions

                            Pill {
                                text: modelData + "m"
                                active: root.activeIdle === modelData
                                tone: root.activeIdle === modelData ? "ink" : "muted"
                                onClicked: root.activeIdle = modelData
                            }
                        }
                    }
                }

                ToggleRow {
                    Layout.fillWidth: true
                    label: "Require passphrase on launch"
                    description: "Prompt for your workspace passphrase every time Kassiber opens."
                    checked: true
                }

                ToggleRow {
                    Layout.fillWidth: true
                    label: "Lock on window close"
                    description: "Clear in-memory decrypted state when the app window is closed."
                    checked: true
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.spacingSm
                    Layout.topMargin: 4

                    ActionButton {
                        variant: "secondary"
                        size: "sm"
                        text: "Lock now"
                    }

                    ActionButton {
                        variant: "ghost"
                        size: "sm"
                        text: "Change passphrase\u2026"
                    }

                    Item { Layout.fillWidth: true }
                }
            }

            // ---------------- Data -----------------------------------------
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 4
                Layout.rightMargin: theme.cardPadding + 4
                spacing: theme.spacingSm + 2

                Loader {
                    Layout.fillWidth: true
                    sourceComponent: sectionHeader
                    onLoaded: item.title = "Data"
                }

                GridLayout {
                    Layout.fillWidth: true
                    columns: 3
                    columnSpacing: theme.spacingSm
                    rowSpacing: theme.spacingSm

                    ActionButton { Layout.fillWidth: true; variant: "secondary"; size: "md"; text: "\u2913  Backup" }
                    ActionButton { Layout.fillWidth: true; variant: "secondary"; size: "md"; text: "\u2912  Restore" }
                    ActionButton { Layout.fillWidth: true; variant: "secondary"; size: "md"; text: "\u22ef  Logs" }
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.topMargin: 6
                    Layout.preferredHeight: importsCol.implicitHeight + theme.cardPadding * 2
                    color: Design.paper(theme)
                    border.color: Design.line(theme)
                    border.width: 1

                    ColumnLayout {
                        id: importsCol
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.leftMargin: theme.cardPadding - 2
                        anchors.rightMargin: theme.cardPadding - 2
                        anchors.topMargin: theme.spacingSm
                        spacing: theme.spacingSm

                        Text {
                            text: "LABELS & IMPORTS \u00b7 WORKSPACE-WIDE"
                            color: Design.ink3(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontMicro
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }

                        GridLayout {
                            Layout.fillWidth: true
                            columns: 3
                            columnSpacing: theme.spacingSm
                            rowSpacing: theme.spacingSm

                            ActionButton { Layout.fillWidth: true; variant: "secondary"; size: "sm"; text: "\u2193  Import BIP-329" }
                            ActionButton { Layout.fillWidth: true; variant: "secondary"; size: "sm"; text: "\u2191  Export BIP-329" }
                            ActionButton { Layout.fillWidth: true; variant: "secondary"; size: "sm"; text: "\u2193  Import CSV" }
                        }
                    }
                }

                Column {
                    Layout.fillWidth: true
                    Layout.topMargin: 6
                    spacing: 2

                    Text {
                        width: parent.width
                        text: "DB  ~/.kassiber/kassiber.db \u00b7 2.4 MB"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                    }

                    Text {
                        width: parent.width
                        text: "Last backup  2026-04-17 23:02 \u00b7 backup_2026-04-17.tar.zst"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                    }
                }
            }

            // ---------------- Sync backends --------------------------------
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 4
                Layout.rightMargin: theme.cardPadding + 4
                spacing: theme.spacingSm - 2

                Loader {
                    Layout.fillWidth: true
                    sourceComponent: sectionHeader
                    onLoaded: item.title = "Sync backends"
                }

                Repeater {
                    model: root.mockBackends

                    delegate: Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: backendRow.implicitHeight + theme.spacingSm * 2
                        color: Design.paperAlt(theme)
                        border.color: Design.line(theme)
                        border.width: 1

                        RowLayout {
                            id: backendRow
                            anchors.fill: parent
                            anchors.leftMargin: theme.spacingSm + 2
                            anchors.rightMargin: theme.spacingSm + 2
                            anchors.topMargin: theme.spacingSm
                            anchors.bottomMargin: theme.spacingSm
                            spacing: theme.spacingSm + 2

                            StatusDot {
                                tone: modelData.on ? "ok" : "muted"
                            }

                            NetworkBadge {
                                network: modelData.net
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData.name
                                    color: Design.ink(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBody
                                    elide: Text.ElideRight
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData.url
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
                                    elide: Text.ElideMiddle
                                }
                            }

                            Text {
                                Layout.preferredWidth: 120
                                text: modelData.health
                                color: modelData.on ? Design.ink2(theme) : Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                horizontalAlignment: Text.AlignRight
                            }

                            Text {
                                Layout.preferredWidth: 48
                                text: modelData.on ? "ACTIVE" : "IDLE"
                                color: modelData.on ? theme.positive : Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontMicro
                                font.weight: Font.Bold
                                font.letterSpacing: 1.2
                                horizontalAlignment: Text.AlignRight
                            }
                        }
                    }
                }

                Button {
                    Layout.fillWidth: true
                    flat: true
                    padding: 0
                    implicitHeight: 34
                    hoverEnabled: true

                    contentItem: Row {
                        anchors.centerIn: parent
                        spacing: theme.spacingSm

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: "+"
                            color: Design.ink2(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontBody
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: "ADD BACKEND"
                            color: Design.ink2(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.letterSpacing: 1.4
                        }
                    }

                    background: Rectangle {
                        color: "transparent"
                        border.color: Design.ink3(theme)
                        border.width: 1
                        // dashed border is not native; rendered as solid with ink3 for now
                    }
                }
            }

            // ---------------- Danger zone ----------------------------------
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 4
                Layout.rightMargin: theme.cardPadding + 4
                Layout.bottomMargin: theme.cardPadding + 4
                spacing: theme.spacingSm + 2

                Loader {
                    Layout.fillWidth: true
                    sourceComponent: sectionHeader
                    onLoaded: item.title = "Danger zone"
                }

                ActionButton {
                    variant: "danger"
                    size: "md"
                    text: "\u26a0  Reset workspace"
                }
            }
        }
    }
}
