import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Static Overview (populated). No business logic; no view-model bindings.
// Every card in a row is Layout.fillHeight so they share the row's max
// implicitHeight. Cards carry Layout.minimumWidth so content never clips when
// the window narrows. Spacing/typography flow from theme semantic tokens.
Item {
    id: root

    property bool hideSensitive: false

    signal requestAddConnection()

    // ---------------------------------------------------------------------
    // Mock data
    // ---------------------------------------------------------------------

    readonly property var mockConnections: [
        { id: "c1", label: "Cold Storage",       balance: "1.2481", tone: "ok" },
        { id: "c2", label: "Multisig 2/3 Vault", balance: "3.0814", tone: "ok" },
        { id: "c3", label: "Home Node (CLN)",    balance: "0.0482", tone: "warn" },
        { id: "c4", label: "Alby Hub",           balance: "0.0021", tone: "idle" },
        { id: "c5", label: "minibits.cash",      balance: "0.0002", tone: "ok" }
    ]

    readonly property var mockFilterRanges: [
        { id: "1w",  label: "1W"  },
        { id: "mtd", label: "MTD" },
        { id: "1m",  label: "1M"  },
        { id: "qtd", label: "QTD" },
        { id: "lq",  label: "LQ"  },
        { id: "ytd", label: "YTD" }
    ]

    readonly property var mockChartRanges: [
        { id: "d",   label: "D"   },
        { id: "w",   label: "W"   },
        { id: "m",   label: "M"   },
        { id: "ytd", label: "YTD" },
        { id: "1y",  label: "1Y"  },
        { id: "5y",  label: "5Y"  },
        { id: "all", label: "ALL" }
    ]

    readonly property var mockChartCurrencies: [
        { id: "btc", label: "\u20bf" },
        { id: "eur", label: "EUR" }
    ]

    readonly property var mockBalanceSeries: [
        0.8, 1.1, 1.6, 1.55, 2.2, 2.4, 2.8, 3.1, 3.6, 4.0, 4.3, 4.38
    ]

    readonly property var mockTransactions: [
        { date: "04-18", type: "Income",   counter: "Invoice \u00b7 ACME GmbH",      sats: "+ 2,450,000",      eur: "+ \u20ac1,749.79" },
        { date: "04-17", type: "Expense",  counter: "Server rental \u00b7 Hetzner",  sats: "\u2212 120,431",    eur: "\u2212 \u20ac86.00" },
        { date: "04-16", type: "Transfer", counter: "Internal transfer",              sats: "\u2212 50,000,000", eur: "\u2212 \u20ac35,710.09" },
        { date: "04-15", type: "Income",   counter: "Client payment \u00b7 LN",      sats: "+ 92,808",          eur: "+ \u20ac66.27" },
        { date: "04-14", type: "Expense",  counter: "Equipment \u00b7 BitcoinStore", sats: "\u2212 890,210",    eur: "\u2212 \u20ac635.71" },
        { date: "04-12", type: "Income",   counter: "Sale \u00b7 Consulting",        sats: "+ 3,800,000",       eur: "+ \u20ac2,713.97" }
    ]

    readonly property var mockBalanceRows: [
        {
            label: "Assets", subtitle: "Resources owned",
            total: "4.38 007 404", negative: false,
            children: [
                { label: "On-chain holdings",  value: "4.32 953 372" },
                { label: "Lightning channels", value: "0.04 821 309" },
                { label: "Cashu (ecash)",      value: "0.00 019 823" },
                { label: "NWC balances",       value: "0.00 213 500" }
            ]
        },
        { label: "Income",      subtitle: "Money earned",           total: "0.75 520 000", negative: false, children: [] },
        { label: "Expenses",    subtitle: "Money spent",            total: "0.02 812 410", negative: true,  children: [] },
        { label: "Liabilities", subtitle: "Debts and obligations",  total: "0.00 000 000", negative: false, children: [] },
        { label: "Equity",      subtitle: "Owner contributions",    total: "3.60 000 000", negative: false, children: [] }
    ]

    // Track expanded Balance parent rows by label
    property var expandedBalanceLabels: ["Assets"]

    readonly property var mockReportTiles: [
        {
            title: "Capital gains",
            subtitle: "FIFO \u00b7 EUR \u00b7 \u00a727a EStG",
            detail: "YTD realized: + \u20ac 42,118.92",
            icon: "\u2197"
        },
        {
            title: "Journal entries",
            subtitle: "Debit / credit \u00b7 double-entry",
            detail: "32 entries \u00b7 YTD",
            icon: "\u2261"
        },
        {
            title: "Balance sheet",
            subtitle: "Assets \u00b7 Liabilities \u00b7 Equity",
            detail: "As of 2026-04-18",
            icon: "\u25ad"
        }
    ]

    // Local static state
    property string chartCurrency: "btc"
    property string chartRange: "ytd"
    property string filterRange: "ytd"

    // Minimum widths so the layout degrades gracefully when the window narrows
    readonly property int balanceFiatMinWidth: 540
    readonly property int connectionsMinWidth: 220
    readonly property int filtersMinWidth: 240
    readonly property int transactionsMinWidth: 540
    readonly property int balancesMinWidth: 360
    readonly property int reportTileMinWidth: 260

    // ---------------------------------------------------------------------
    // Layout
    // ---------------------------------------------------------------------

    ScrollView {
        anchors.fill: parent
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap

            // -----------------------------------------------------------------
            // Row 1 — Balance over time | Connections | Filters | Fiat · EUR
            // -----------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding
                columns: 4
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // ----- Balance over time -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 16
                    Layout.minimumWidth: 360
                    title: "Balance over time"

                    action: Component {
                        Row {
                            spacing: theme.spacingSm - 2

                            SegmentedControl {
                                anchors.verticalCenter: parent.verticalCenter
                                model: root.mockChartCurrencies
                                currentId: root.chartCurrency
                                itemHeight: theme.controlHeightSm
                                fontPixelSize: theme.fontCaption
                                onActivated: root.chartCurrency = id
                            }

                            Button {
                                anchors.verticalCenter: parent.verticalCenter
                                flat: true
                                padding: 0
                                implicitWidth: theme.controlHeightSm - 2
                                implicitHeight: theme.controlHeightSm - 2

                                contentItem: Text {
                                    anchors.fill: parent
                                    text: "\u21f1"
                                    color: Design.ink2(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
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

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: theme.gridGap

                        BalanceChart {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 160
                            series: root.mockBalanceSeries
                            currency: root.chartCurrency
                        }

                        SegmentedControl {
                            Layout.fillWidth: true
                            model: root.mockChartRanges
                            currentId: root.chartRange
                            itemHeight: theme.controlHeightMd
                            fillWidth: true
                            borderOuter: false
                            onActivated: root.chartRange = id
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: theme.spacingSm + 6

                            KV {
                                label: "Current"
                                value: root.hideSensitive
                                    ? "\u2022 \u2022 \u2022 \u2022"
                                    : (root.chartCurrency === "eur"
                                        ? "\u20ac 312,842.77"
                                        : "4.38000181 \u20bf")
                            }

                            KV {
                                label: "\u0394 YTD"
                                value: root.chartCurrency === "eur"
                                    ? "+ \u20ac 118,420.50"
                                    : "+ 1.88000000 \u20bf"
                                valueColor: theme.positive
                            }

                            KV {
                                label: "\u0394 30d"
                                value: root.chartCurrency === "eur"
                                    ? "\u2212 \u20ac 1,890.20"
                                    : "\u2212 0.03 \u20bf"
                                valueColor: Design.accent(theme)
                            }

                            Item { Layout.fillWidth: true }
                        }
                    }
                }

                // ----- Connections (pad=false, footer anchored to bottom) -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 10
                    Layout.minimumWidth: root.connectionsMinWidth
                    title: "Connections"
                    pad: false

                    action: Component {
                        Row {
                            spacing: theme.spacingSm - 2

                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                text: String(root.mockConnections.length)
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontMicro
                                font.letterSpacing: 0.8
                            }

                            Button {
                                anchors.verticalCenter: parent.verticalCenter
                                flat: true
                                padding: 0
                                implicitWidth: theme.controlHeightSm - 2
                                implicitHeight: theme.controlHeightSm - 2

                                contentItem: Text {
                                    anchors.fill: parent
                                    text: "\u21bb"
                                    color: Design.ink2(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
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

                    // Connections body: list rows + footer all as direct
                    // ColumnLayout children. ColumnLayout.implicitHeight flows
                    // up through the Card, so the "Add connection" row is
                    // always a real child of the card rather than an absolutely-
                    // positioned sibling.
                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Repeater {
                            model: root.mockConnections

                            ConnectionRow {
                                Layout.fillWidth: true
                                Layout.preferredHeight: theme.rowHeightCompact + 2
                                connectionLabel: modelData.label
                                balanceLabel: modelData.balance
                                statusTone: modelData.tone
                                borderTop: index > 0
                                hideSensitive: root.hideSensitive
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: theme.rowHeightCompact + 2
                            color: Design.paper(theme)

                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                height: 1
                                color: Design.line(theme)
                            }

                            Button {
                                anchors.fill: parent
                                flat: true
                                padding: 0
                                hoverEnabled: true
                                onClicked: root.requestAddConnection()

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
                                        text: "Add connection"
                                        color: Design.ink(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBody
                                        font.weight: Font.Medium
                                    }
                                }

                                background: Rectangle { color: "transparent" }
                            }
                        }
                    }
                }

                // ----- Filters -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 10
                    Layout.minimumWidth: root.filtersMinWidth
                    title: "Filters"

                    action: Component {
                        Text {
                            text: "\u21ba RESET"
                            color: Design.ink2(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.letterSpacing: 0.8
                        }
                    }

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: theme.gridGap

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: theme.spacingSm - 2

                            DateInput {
                                Layout.fillWidth: true
                                label: "From"
                                value: "2026-01-01"
                            }

                            DateInput {
                                Layout.fillWidth: true
                                label: "To"
                                value: "2026-04-18"
                            }
                        }

                        Flow {
                            Layout.fillWidth: true
                            spacing: theme.spacingXs

                            Repeater {
                                model: root.mockFilterRanges

                                Pill {
                                    text: modelData.label
                                    active: root.filterRange === modelData.id
                                    tone: root.filterRange === modelData.id ? "ink" : "muted"
                                    onClicked: root.filterRange = modelData.id
                                }
                            }
                        }

                        Column {
                            Layout.fillWidth: true
                            spacing: theme.spacingSm - 2

                            Text {
                                text: "ACCOUNT"
                                color: Design.ink3(theme)
                                font.family: Design.sans()
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.4
                            }

                            Rectangle {
                                width: parent.width
                                height: theme.controlHeightLg - 2
                                color: Design.paperAlt(theme)
                                border.color: Design.line(theme)
                                border.width: 1

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    anchors.left: parent.left
                                    anchors.leftMargin: theme.spacingSm + 2
                                    text: "All accounts"
                                    color: Design.ink(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontBody
                                }

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    anchors.right: parent.right
                                    anchors.rightMargin: theme.spacingSm + 2
                                    text: "\u25be"
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
                                }
                            }
                        }
                    }
                }

                // ----- Fiat \u00b7 EUR -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 11
                    Layout.minimumWidth: 220
                    title: "Fiat \u00b7 EUR"

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: theme.spacingSm + 4

                        Column {
                            Layout.fillWidth: true
                            spacing: 2

                            Text {
                                text: "CURRENT BTC/EUR"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontMicro
                                font.letterSpacing: 1.2
                            }

                            Text {
                                text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : "\u20ac 71,420.18"
                                color: Design.ink(theme)
                                font.family: Design.sans()
                                font.pixelSize: theme.fontHeadingXl
                                font.letterSpacing: -0.2
                            }

                            Text {
                                text: "+ 1.42 %  \u00b7  24h"
                                color: theme.positive
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                            }
                        }

                        Rule { Layout.fillWidth: true }

                        GridLayout {
                            Layout.fillWidth: true
                            columns: 2
                            columnSpacing: theme.gridGap
                            rowSpacing: theme.gridGap

                            KV {
                                label: "Cost basis"
                                value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : "\u20ac 198,502.40"
                                mono: false
                            }
                            KV {
                                label: "Market value"
                                value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : "\u20ac 312,842.77"
                                mono: false
                            }
                            KV {
                                label: "Unrealized P/L"
                                value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : "+ \u20ac 114,340.37"
                                valueColor: theme.positive
                                mono: false
                            }
                            KV {
                                label: "Realized YTD"
                                value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : "+ \u20ac 42,118.92"
                                valueColor: theme.positive
                                mono: false
                            }
                        }
                    }
                }
            }

            // -----------------------------------------------------------------
            // Row 2 — Transactions preview | Balances
            // -----------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // ----- Transactions preview -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 16
                    Layout.minimumWidth: root.transactionsMinWidth
                    title: "Transactions"
                    pad: false

                    action: Component {
                        Row {
                            spacing: theme.spacingSm + 4

                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                text: String(root.mockTransactions.length) + " ENTRIES"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 0.8
                            }

                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                text: "OPEN ALL \u2192"
                                color: Design.ink(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.0
                            }
                        }
                    }

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

                                Text { Layout.preferredWidth: 50;  text: "DATE";         color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Text { Layout.preferredWidth: 80;  text: "TYPE";         color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Text { Layout.fillWidth: true;     text: "COUNTERPARTY"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Text { Layout.preferredWidth: 130; text: "SATS";         color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                Text { Layout.preferredWidth: 110; text: "\u20ac";       color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                            }
                        }

                        Repeater {
                            model: root.mockTransactions

                            delegate: Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: theme.rowHeightCompact
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
                                        Layout.preferredWidth: 50
                                        text: modelData.date
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                    }

                                    Text {
                                        Layout.preferredWidth: 80
                                        text: modelData.type.toUpperCase()
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontMicro
                                        font.letterSpacing: 1.0
                                        color: {
                                            if (modelData.type === "Income") return theme.typeIncome
                                            if (modelData.type === "Expense") return theme.typeExpense
                                            if (modelData.type === "Transfer") return theme.typeTransfer
                                            if (modelData.type === "Swap") return theme.typeSwap
                                            if (modelData.type === "Mint") return theme.typeMint
                                            if (modelData.type === "Melt") return theme.typeMelt
                                            if (modelData.type === "Fee") return theme.typeFee
                                            return Design.ink2(theme)
                                        }
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData.counter
                                        color: Design.ink(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBody
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        Layout.preferredWidth: 130
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.sats
                                        color: modelData.sats.indexOf("+") === 0 ? theme.positive : Design.ink(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                    }

                                    Text {
                                        Layout.preferredWidth: 110
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.eur
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                    }
                                }
                            }
                        }
                    }
                }

                // ----- Balances -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 15
                    Layout.minimumWidth: root.balancesMinWidth
                    title: "Balances"

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Repeater {
                            model: root.mockBalanceRows

                            BalanceRow {
                                Layout.fillWidth: true
                                label: modelData.label
                                subtitle: modelData.subtitle
                                totalLabel: modelData.total
                                negative: modelData.negative
                                bottomBorder: index < root.mockBalanceRows.length - 1
                                hideSensitive: root.hideSensitive
                                children_: modelData.children
                                expanded: root.expandedBalanceLabels.indexOf(modelData.label) !== -1
                                onToggled: {
                                    var current = root.expandedBalanceLabels.slice()
                                    var idx = current.indexOf(modelData.label)
                                    if (idx === -1) {
                                        current.push(modelData.label)
                                    } else {
                                        current.splice(idx, 1)
                                    }
                                    root.expandedBalanceLabels = current
                                }
                            }
                        }
                    }
                }
            }

            // -----------------------------------------------------------------
            // Row 3 — Report tiles
            // -----------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                columns: 3
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                Repeater {
                    model: root.mockReportTiles

                    ReportTile {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        Layout.minimumWidth: root.reportTileMinWidth
                        title: modelData.title
                        subtitle: modelData.subtitle
                        detail: modelData.detail
                        iconGlyph: modelData.icon
                    }
                }
            }
        }
    }
}
