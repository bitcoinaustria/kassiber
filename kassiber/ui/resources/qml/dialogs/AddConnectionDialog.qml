import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Connection-type picker modal (step 1 of Add Connection).
// Sticky intro at top, scrollable grouped list, sticky watch-only note at bottom.
Dialog {
    id: root
    title: "Add a connection"
    modal: true
    width: 760
    padding: 0
    standardButtons: Dialog.NoButton

    readonly property var sections: [
        {
            label: "Self-custody \u00b7 On-chain",
            items: [
                { id: "xpub",       name: "XPub",       desc: "Single-sig on-chain watch" },
                { id: "descriptor", name: "Descriptor", desc: "Multisig wallet descriptor" }
            ]
        },
        {
            label: "Lightning",
            items: [
                { id: "core-ln", name: "Core Lightning", desc: "CLN node RPC" },
                { id: "lnd",     name: "LND",            desc: "Lightning Network Daemon" },
                { id: "nwc",     name: "NWC",            desc: "Nostr Wallet Connect" }
            ]
        },
        {
            label: "Services \u00b7 Merchant",
            items: [
                { id: "btcpay", name: "BTCPay Server", desc: "Merchant API \u00b7 store read-key" },
                { id: "cashu",  name: "Cashu",         desc: "Ecash mint wallet" }
            ]
        },
        {
            label: "Exchanges \u00b7 Read-only API",
            items: [
                { id: "kraken",   name: "Kraken",   desc: "Read-only API key" },
                { id: "bitstamp", name: "Bitstamp", desc: "Read-only API key" },
                { id: "coinbase", name: "Coinbase", desc: "Read-only API key" },
                { id: "bitpanda", name: "Bitpanda", desc: "Read-only API key \u00b7 Austrian" },
                { id: "river",    name: "River",    desc: "Read-only API key" },
                { id: "strike",   name: "Strike",   desc: "Read-only API key" }
            ]
        },
        {
            label: "File",
            items: [
                { id: "csv",    name: "CSV import",    desc: "One-shot, from file" },
                { id: "bip329", name: "BIP-329 labels", desc: "Import labels \u00b7 JSONL" }
            ]
        }
    ]

    signal kindPicked(string kind)

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

    contentItem: ColumnLayout {
        spacing: 0

        // Sticky intro
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: intro.implicitHeight + 24
            color: Design.paper(theme)

            Text {
                id: intro
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: theme.cardPadding + 4
                anchors.rightMargin: theme.cardPadding + 4
                text: "Kassiber is watch-only. Keys never leave your machine."
                color: Design.ink2(theme)
                font.family: Design.sans()
                font.pixelSize: theme.fontBodyStrong
                wrapMode: Text.WordWrap
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Design.line(theme)
            }
        }

        // Scrollable section list
        ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: 440
            clip: true
            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

            ColumnLayout {
                width: root.availableWidth
                spacing: theme.gridGap + 6

                Repeater {
                    model: root.sections

                    delegate: ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: theme.cardPadding + 4
                        Layout.rightMargin: theme.cardPadding + 4
                        Layout.topMargin: index === 0 ? theme.gridGap : 0
                        spacing: theme.spacingSm + 2

                        // Section header: label + rule + count
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: theme.spacingSm + 2

                            Text {
                                text: modelData.label.toUpperCase()
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.Bold
                                font.letterSpacing: 1.6
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: 1
                                color: Design.line(theme)
                            }

                            Text {
                                text: (modelData.items.length < 10 ? "0" : "") + modelData.items.length
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.Bold
                                font.letterSpacing: 1.4
                            }
                        }

                        // 2-col row grid
                        GridLayout {
                            Layout.fillWidth: true
                            columns: 2
                            columnSpacing: theme.spacingSm - 2
                            rowSpacing: theme.spacingSm - 2

                            Repeater {
                                model: modelData.items

                                KindPickerRow {
                                    Layout.fillWidth: true
                                    connectionName: modelData.name
                                    description: modelData.desc
                                    onClicked: root.kindPicked(modelData.id)
                                }
                            }
                        }
                    }
                }

                Item { Layout.preferredHeight: theme.gridGap }
            }
        }

        // Sticky footer note
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: noteContent.implicitHeight + 28
            color: Design.paperAlt(theme)

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: Design.line(theme)
            }

            RowLayout {
                id: noteContent
                anchors.fill: parent
                anchors.leftMargin: theme.cardPadding + 4
                anchors.rightMargin: theme.cardPadding + 4
                spacing: theme.spacingSm + 2

                Text {
                    Layout.preferredWidth: 14
                    text: "\u25a1"
                    color: Design.accent(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontBodyStrong
                }

                Text {
                    Layout.fillWidth: true
                    text: "Watch-only by design. Kassiber imports history via extended public keys, descriptors, or read-only API credentials. No private keys or withdrawal permissions ever touch this machine through Kassiber."
                    color: Design.ink2(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontBodySmall
                    wrapMode: Text.WordWrap
                }
            }
        }
    }
}
