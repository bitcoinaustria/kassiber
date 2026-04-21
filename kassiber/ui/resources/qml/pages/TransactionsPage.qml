import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import Qt5Compat.GraphicalEffects

import "../components"
import "../components/Design.js" as Design

// Static Transactions ledger. Reuses existing Card/Pill/TypeBadge/ActionButton
// primitives, plus the new SearchField and TagChip. Mock data inline.
Item {
    id: root

    property bool hideSensitive: false

    // Mock data -------------------------------------------------------------
    readonly property var mockTransactions: [
        { id: "tx1",  date: "2026-04-18 14:22", type: "Income",        account: "Cold Storage",                         counter: "Invoice \u00b7 ACME GmbH",            sats: "+ 2,450,000",      rate: "\u20ac 71,420.18", eur: "+ \u20ac 1,749.79",  tag: "Revenue",      conf: 41  },
        { id: "tx2",  date: "2026-04-17 09:08", type: "Expense",       account: "Home Node (CLN)",                      counter: "Server rental \u00b7 Hetzner",        sats: "\u2212 120,431",   rate: "\u20ac 71,432.10", eur: "\u2212 \u20ac 86.00",  tag: "Hosting",      conf: 140 },
        { id: "tx3",  date: "2026-04-16 16:51", type: "Transfer",      account: "Cold Storage \u2192 Vault",            counter: "Internal transfer",                    sats: "\u2212 50,000,000",rate: "\u20ac 71,420.18", eur: "\u2212 \u20ac 35,710.09", tag: "Transfer",  conf: 220 },
        { id: "tx4",  date: "2026-04-15 11:14", type: "Income",        account: "NWC \u00b7 Alby",                      counter: "Client payment \u00b7 LN",             sats: "+ 92,808",         rate: "\u20ac 71,398.42", eur: "+ \u20ac 66.27",       tag: "Revenue",      conf: 1   },
        { id: "tx5",  date: "2026-04-14 22:02", type: "Expense",       account: "Multisig Vault",                       counter: "Equipment \u00b7 BitcoinStore",        sats: "\u2212 890,210",   rate: "\u20ac 71,412.00", eur: "\u2212 \u20ac 635.71",     tag: "Capex",        conf: 420 },
        { id: "tx6",  date: "2026-04-12 08:30", type: "Income",        account: "Cold Storage",                         counter: "Sale \u00b7 Consulting",               sats: "+ 3,800,000",      rate: "\u20ac 71,420.18", eur: "+ \u20ac 2,713.97",    tag: "Revenue",      conf: 612 },
        { id: "tx7",  date: "2026-04-11 19:45", type: "Expense",       account: "Cashu \u00b7 minibits",                counter: "Coffee",                                sats: "\u2212 8,400",     rate: "\u20ac 71,428.57", eur: "\u2212 \u20ac 6.00",   tag: "Meals",        conf: 1   },
        { id: "tx8",  date: "2026-04-09 10:00", type: "Fee",           account: "Home Node (CLN)",                      counter: "Channel open",                          sats: "\u2212 18,210",    rate: "\u20ac 71,445.91", eur: "\u2212 \u20ac 13.01",  tag: "Bank fees",    conf: 380 },
        { id: "tx9",  date: "2026-04-07 13:12", type: "Income",        account: "Multisig Vault",                       counter: "Invoice \u00b7 Globex AG",             sats: "+ 1,210,000",      rate: "\u20ac 71,420.00", eur: "+ \u20ac 864.18",      tag: "Revenue",      conf: 820 },
        { id: "tx10", date: "2026-04-06 15:30", type: "Swap",          account: "NWC \u00b7 Alby \u2192 Cashu \u00b7 minibits", counter: "LN \u2192 ecash swap",        sats: "+ 500,000",        rate: "\u20ac 71,420.00", eur: "+ \u20ac 357.10",      tag: "Swap",         conf: 1   },
        { id: "tx11", date: "2026-04-05 11:08", type: "Swap",          account: "Multisig Vault \u2192 Home Node (CLN)", counter: "Submarine swap \u00b7 on-chain \u2192 LN", sats: "+ 2,000,000",   rate: "\u20ac 71,420.00", eur: "+ \u20ac 1,428.40",    tag: "Swap",         conf: 12  },
        { id: "tx12", date: "2026-04-03 09:22", type: "Consolidation", account: "Cold Storage",                         counter: "12 UTXOs \u2192 1",                    sats: "\u2212 42,180",    rate: "\u20ac 71,432.00", eur: "\u2212 \u20ac 30.13",  tag: "Consolidation",conf: 210 },
        { id: "tx13", date: "2026-03-30 18:44", type: "Consolidation", account: "Multisig Vault",                       counter: "8 UTXOs \u2192 1",                     sats: "\u2212 58,900",    rate: "\u20ac 71,432.00", eur: "\u2212 \u20ac 42.08",  tag: "Consolidation",conf: 980 },
        { id: "tx14", date: "2026-03-28 12:10", type: "Rebalance",     account: "Home Node (CLN)",                      counter: "Circular rebalance \u00b7 LN",         sats: "\u2212 2,140",     rate: "\u20ac 71,432.00", eur: "\u2212 \u20ac 1.53",   tag: "Rebalance",    conf: 1   },
        { id: "tx15", date: "2026-03-25 20:15", type: "Mint",          account: "Cashu \u00b7 minibits",                counter: "Mint ecash from LN",                    sats: "+ 100,000",        rate: "\u20ac 71,420.00", eur: "+ \u20ac 71.42",       tag: "Mint",         conf: 1   },
        { id: "tx16", date: "2026-03-24 17:40", type: "Melt",          account: "Cashu \u00b7 minibits",                counter: "Melt ecash to LN",                      sats: "\u2212 50,000",    rate: "\u20ac 71,420.00", eur: "\u2212 \u20ac 35.71", tag: "Melt",         conf: 1   }
    ]

    readonly property var mockTypes: [
        "all", "Income", "Expense", "Transfer", "Swap",
        "Consolidation", "Rebalance", "Mint", "Melt", "Fee"
    ]

    // Local static state
    property string searchQuery: ""
    property string activeType: "all"

    // Column widths (minimums, so narrowing doesn't overflow)
    readonly property int colDate: 130
    readonly property int colType: 90
    readonly property int colAccount: 180
    readonly property int colTag: 110
    readonly property int colSats: 130
    readonly property int colRate: 110
    readonly property int colEur: 120
    readonly property int colConf: 50

    function typeColor(t) {
        if (t === "Income") return theme.typeIncome
        if (t === "Expense") return theme.typeExpense
        if (t === "Transfer") return theme.typeTransfer
        if (t === "Swap") return theme.typeSwap
        if (t === "Consolidation") return theme.typeConsolidation
        if (t === "Rebalance") return theme.typeRebalance
        if (t === "Mint") return theme.typeMint
        if (t === "Melt") return theme.typeMelt
        if (t === "Fee") return theme.typeFee
        return Design.ink3(theme)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: theme.gridGap

            // ------------------------------------------------------------------
            // Header: eyebrow + title (left), action cluster (right)   [fixed]
            // ------------------------------------------------------------------

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding
                spacing: theme.spacingSm + 2

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Text {
                        text: "LEDGER  \u00b7  " + root.mockTransactions.length + " ENTRIES  \u00b7  2026"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.letterSpacing: 1.4
                    }

                    Text {
                        text: "Transactions"
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontDisplay - 4
                        font.weight: Font.DemiBold
                        font.letterSpacing: -0.4
                    }
                }

                ActionButton {
                    variant: "secondary"
                    size: "sm"
                    text: "\u2193 Import labels"
                }

                ActionButton {
                    variant: "secondary"
                    size: "sm"
                    text: "\u2191 Export labels"
                }

                Rectangle {
                    Layout.preferredWidth: 1
                    Layout.preferredHeight: 22
                    color: Design.line(theme)
                }

                ActionButton {
                    variant: "secondary"
                    size: "sm"
                    text: "\u2913 CSV"
                }

                ActionButton {
                    variant: "secondary"
                    size: "sm"
                    text: "\u2913 JSON"
                }

                ActionButton {
                    variant: "primary"
                    size: "sm"
                    text: "+ Manual entry"
                }
            }

            // ------------------------------------------------------------------
            // Filter strip (outlined bar: search + type pills)          [fixed]
            // ------------------------------------------------------------------

            Rectangle {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.preferredHeight: row.implicitHeight + theme.spacingSm * 2
                color: Design.paperAlt(theme)
                border.color: Design.line(theme)
                border.width: 1

                RowLayout {
                    id: row
                    anchors.fill: parent
                    anchors.leftMargin: theme.cardPadding - 2
                    anchors.rightMargin: theme.cardPadding - 2
                    anchors.topMargin: theme.spacingSm
                    anchors.bottomMargin: theme.spacingSm
                    spacing: theme.spacingSm + 4

                    SearchField {
                        Layout.fillWidth: true
                        Layout.preferredWidth: 360
                        Layout.maximumWidth: 480
                        text: root.searchQuery
                        placeholderText: "Search counterparty, tag, account\u2026"
                    }

                    Rectangle {
                        Layout.preferredWidth: 1
                        Layout.preferredHeight: 20
                        color: Design.line(theme)
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: theme.spacingXs

                        Repeater {
                            model: root.mockTypes

                            Pill {
                                text: modelData === "all" ? "all" : modelData
                                active: root.activeType === modelData
                                tone: root.activeType === modelData ? "ink" : "muted"
                                onClicked: root.activeType = modelData
                            }
                        }
                    }
                }
            }

        // ------------------------------------------------------------------
        // Table                                         [list scrolls, header sticky]
        // ------------------------------------------------------------------

        ListView {
            id: txList
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.leftMargin: theme.pagePadding
            Layout.rightMargin: theme.pagePadding
            Layout.bottomMargin: theme.pagePadding
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            model: root.mockTransactions
            headerPositioning: ListView.OverlayHeader

            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AsNeeded
            }

            header: Rectangle {
                width: txList.width
                height: theme.rowHeightDefault
                color: Design.paper(theme)
                z: 2

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Design.ink(theme)
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: theme.cardPadding
                    anchors.rightMargin: theme.cardPadding
                    spacing: theme.spacingSm + 4

                    Text { Layout.preferredWidth: root.colDate;    text: "DATE \u00b7 TIME"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colType;    text: "TYPE";             color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colAccount; text: "ACCOUNT";          color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.fillWidth: true;                 text: "COUNTERPARTY";     color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colTag;     text: "TAG";              color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colSats;    text: "SATS";             color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                    Text { Layout.preferredWidth: root.colRate;    text: "BTC/EUR RATE";     color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                    Text { Layout.preferredWidth: root.colEur;     text: "EUR";              color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                    Text { Layout.preferredWidth: root.colConf;    text: "CONF";             color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                }
            }

            delegate: Rectangle {
                width: txList.width
                height: theme.rowHeightDefault
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

                    // Type badge (outlined, tone-colored)
                    Item {
                        Layout.preferredWidth: root.colType
                        Layout.preferredHeight: 18

                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: typeText.implicitWidth + 12
                            height: typeText.implicitHeight + 4
                            color: "transparent"
                            border.color: root.typeColor(modelData.type)
                            border.width: 1

                            Text {
                                id: typeText
                                anchors.centerIn: parent
                                text: modelData.type.toUpperCase()
                                color: root.typeColor(modelData.type)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontMicro
                                font.letterSpacing: 1.0
                                font.weight: Font.DemiBold
                            }
                        }
                    }

                    Text {
                        Layout.preferredWidth: root.colAccount
                        text: modelData.account
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.fillWidth: true
                        text: modelData.counter
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                        elide: Text.ElideRight
                    }

                    // Tag chip
                    TagChip {
                        Layout.preferredWidth: root.colTag
                        Layout.alignment: Qt.AlignVCenter
                        label: modelData.tag
                    }

                    Text {
                        Layout.preferredWidth: root.colSats
                        text: modelData.sats
                        color: modelData.sats.indexOf("+") === 0 ? theme.positive : Design.ink(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                        horizontalAlignment: Text.AlignRight
                        layer.enabled: root.hideSensitive
                        layer.effect: FastBlur { radius: 48 }
                    }

                    Text {
                        Layout.preferredWidth: root.colRate
                        text: modelData.rate
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                        horizontalAlignment: Text.AlignRight
                    }

                    Text {
                        Layout.preferredWidth: root.colEur
                        text: modelData.eur
                        color: Design.ink2(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                        horizontalAlignment: Text.AlignRight
                        layer.enabled: root.hideSensitive
                        layer.effect: FastBlur { radius: 48 }
                    }

                    Text {
                        Layout.preferredWidth: root.colConf
                        text: modelData.conf
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
