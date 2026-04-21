import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Static Connection Detail view. Static mock data inline, no view-model
// bindings. Reuses Card / KV / ActionButton / ProtocolIcon / StatTile.
Item {
    id: root

    property bool hideSensitive: false

    signal requestAddConnection()
    signal requestBack()

    // Mock data --------------------------------------------------------------

    readonly property var mockConnection: ({
        label: "Cold Storage",
        kind: "xpub",
        protocol: "XPUB",
        balanceBtc: "1.24810472 \u20bf",
        balanceFiat: "\u20ac 89,162.05",
        addresses: 142,
        lastSync: "2m ago",
        lastSyncTone: "ok",
        gap: 10,
        created: "2026-03-02 10:14",
        derivation: "m / 84' / 0' / 0'",
        backend: "mempool.space",
        kassiberId: "conn_01HX2..3f7k"
    })

    readonly property var mockTxs: [
        { date: "04-18 14:22", type: "Income",   sats: "+ 2,450,000",       eur: "+ \u20ac 1,749.79",    conf: 41  },
        { date: "04-12 08:30", type: "Income",   sats: "+ 3,800,000",       eur: "+ \u20ac 2,713.97",    conf: 612 },
        { date: "04-03 09:22", type: "Expense",  sats: "\u2212 42,180",     eur: "\u2212 \u20ac 30.13",  conf: 210 },
        { date: "03-28 12:10", type: "Transfer", sats: "\u2212 1,210,000",  eur: "\u2212 \u20ac 864.18", conf: 320 },
        { date: "03-20 18:44", type: "Income",   sats: "+ 4,250,000",       eur: "+ \u20ac 3,035.36",    conf: 980 }
    ]

    readonly property var mockAddresses: [
        { address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", path: "m/84'/0'/0'/0/0" },
        { address: "bc1q9d4ywgfnd8h43da5tpcxcn6ajv590cg6d3tg6a", path: "m/84'/0'/0'/0/1" },
        { address: "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh", path: "m/84'/0'/0'/0/2" },
        { address: "bc1qzk6qk7mxeq8p0we45e24gm20q6r4xrrjy0wl2g", path: "m/84'/0'/0'/0/3" },
        { address: "bc1q3n7tz3qv4wtpgd6d8h3l8l6j3rhr8z2ydkn3g2", path: "m/84'/0'/0'/0/4" }
    ]

    readonly property int colDate: 80
    readonly property int colType: 90
    readonly property int colSats: 140
    readonly property int colEur: 120
    readonly property int colConf: 50

    function typeColor(t) {
        if (t === "Income") return theme.typeIncome
        if (t === "Expense") return theme.typeExpense
        if (t === "Transfer") return theme.typeTransfer
        if (t === "Swap") return theme.typeSwap
        return Design.ink3(theme)
    }

    ScrollView {
        anchors.fill: parent
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap

            // ------------------------------------------------------------------
            // Header
            // ------------------------------------------------------------------

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding
                spacing: theme.spacingSm + 6

                Button {
                    flat: true
                    padding: 0
                    implicitWidth: 32
                    implicitHeight: 32
                    hoverEnabled: true
                    onClicked: root.requestBack()

                    contentItem: Text {
                        anchors.fill: parent
                        text: "\u2039"
                        color: Design.ink(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontHeadingSm
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }

                    background: Rectangle {
                        color: "transparent"
                        border.color: Design.line(theme)
                        border.width: 1
                    }
                }

                ProtocolIcon {
                    kind: root.mockConnection.kind
                    size: 40
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Text {
                        text: root.mockConnection.protocol + "  \u00b7  CONNECTION"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.letterSpacing: 1.4
                    }

                    Text {
                        text: root.mockConnection.label
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontDisplay - 2
                        font.weight: Font.DemiBold
                        font.letterSpacing: -0.4
                    }
                }

                ActionButton { variant: "secondary"; size: "sm"; text: "\u27f3 Sync" }
                ActionButton { variant: "secondary"; size: "sm"; text: "\u2193 Import labels" }
                ActionButton { variant: "secondary"; size: "sm"; text: "\u2191 Export labels" }

                Rectangle {
                    Layout.preferredWidth: 1
                    Layout.preferredHeight: 22
                    color: Design.line(theme)
                }

                ActionButton { variant: "ghost"; size: "sm"; text: "Edit" }
                ActionButton { variant: "danger"; size: "sm"; text: "Remove" }
            }

            // ------------------------------------------------------------------
            // 4-stat tile row
            // ------------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 4
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 220
                    label: "Balance"
                    value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockConnection.balanceBtc
                    sub: root.mockConnection.balanceFiat
                }

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 160
                    label: "Addresses"
                    value: String(root.mockConnection.addresses)
                    sub: "derived"
                }

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 160
                    label: "Last sync"
                    value: root.mockConnection.lastSync
                    sub: "synced"
                    valueColor: theme.positive
                }

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 160
                    label: "Gap limit"
                    value: String(root.mockConnection.gap)
                    sub: "unused window"
                }
            }

            // ------------------------------------------------------------------
            // 2-col row: Recent transactions | (Connection details + Derived addresses)
            // ------------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // ----- Recent transactions -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 14
                    Layout.minimumWidth: 520
                    title: "Recent transactions"
                    pad: false

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: theme.rowHeightDefault - 6
                            color: "transparent"

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
                                spacing: theme.spacingSm + 4

                                Text { Layout.preferredWidth: root.colDate; text: "DATE";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Text { Layout.preferredWidth: root.colType; text: "TYPE";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Item { Layout.fillWidth: true }
                                Text { Layout.preferredWidth: root.colSats; text: "SATS";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                Text { Layout.preferredWidth: root.colEur;  text: "\u20ac"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                Text { Layout.preferredWidth: root.colConf; text: "CONF";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                            }
                        }

                        Repeater {
                            model: root.mockTxs

                            delegate: Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: theme.rowHeightDefault
                                color: "transparent"

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
                                    spacing: theme.spacingSm + 4

                                    Text {
                                        Layout.preferredWidth: root.colDate
                                        text: modelData.date
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                    }

                                    Text {
                                        Layout.preferredWidth: root.colType
                                        text: modelData.type.toUpperCase()
                                        color: root.typeColor(modelData.type)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontMicro
                                        font.weight: Font.DemiBold
                                        font.letterSpacing: 1.0
                                    }

                                    Item { Layout.fillWidth: true }

                                    Text {
                                        Layout.preferredWidth: root.colSats
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.sats
                                        color: modelData.sats.indexOf("+") === 0 ? theme.positive : Design.ink(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                    }

                                    Text {
                                        Layout.preferredWidth: root.colEur
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.eur
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                    }

                                    Text {
                                        Layout.preferredWidth: root.colConf
                                        text: String(modelData.conf)
                                        color: Design.ink3(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                    }
                                }
                            }
                        }
                    }
                }

                // ----- Right column: Connection details + Derived addresses -----
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 10
                    Layout.minimumWidth: 320
                    spacing: theme.gridGap

                    Card {
                        Layout.fillWidth: true
                        title: "Connection details"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm + 4

                            KV { label: "Label";        value: root.mockConnection.label;        mono: false }
                            KV { label: "Protocol";     value: root.mockConnection.protocol }
                            KV { label: "Derivation";   value: root.mockConnection.derivation }
                            KV { label: "Backend";      value: root.mockConnection.backend;      mono: false }
                            KV { label: "Created";      value: root.mockConnection.created }
                            KV { label: "Kassiber ID";  value: root.mockConnection.kassiberId }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Derived addresses"
                        pad: false

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: 0

                            Repeater {
                                model: root.mockAddresses

                                delegate: Rectangle {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: theme.rowHeightCompact
                                    color: "transparent"

                                    Rectangle {
                                        visible: index > 0
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.top: parent.top
                                        height: 1
                                        color: Design.line(theme)
                                    }

                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: theme.cardPadding
                                        anchors.rightMargin: theme.cardPadding

                                        Text {
                                            Layout.fillWidth: true
                                            text: root.hideSensitive
                                                ? "\u2022 \u2022 \u2022 \u2022"
                                                : modelData.address.slice(0, 28) + "\u2026"
                                            color: Design.ink(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontCaption
                                            elide: Text.ElideRight
                                        }

                                        Text {
                                            text: modelData.path
                                            color: Design.ink3(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontCaption
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
