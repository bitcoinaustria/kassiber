import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import Qt5Compat.GraphicalEffects

import "../components"
import "../components/Design.js" as Design

Item {
    id: root

    property bool hideSensitive: false

    signal requestAddConnection()

    readonly property var metricRows: dashboardVM ? dashboardVM.overviewMetrics : []
    readonly property var highlightRows: dashboardVM ? dashboardVM.overviewHighlights : []
    readonly property var connectionRows: connectionsVM ? connectionsVM.items : []
    readonly property var transactionRows: transactionsVM ? transactionsVM.items : []
    readonly property var visibleTransactionRows: transactionRows ? transactionRows.slice(0, 8) : []
    readonly property var reportRows: reportsVM ? reportsVM.items : []
    readonly property var noticeRows: dashboardVM ? dashboardVM.notices : []

    function toneColor(tone) {
        if (tone === "ok") return theme.positive
        if (tone === "warn") return Design.accent(theme)
        return Design.ink(theme)
    }

    // Empty state: centered hero panel (shown when the profile has no imported data)
    Item {
        anchors.fill: parent
        visible: !dashboardVM.hasData

        ColumnLayout {
            anchors.centerIn: parent
            width: Math.min(560, parent.width - 80)
            spacing: 28

            Grid {
                Layout.alignment: Qt.AlignHCenter
                rows: 3
                columns: 4
                rowSpacing: 10
                columnSpacing: 10

                Repeater {
                    model: 12

                    delegate: Rectangle {
                        width: 54
                        height: 20
                        color: "transparent"
                        border.color: Design.line(theme)
                        border.width: 1
                        radius: theme.radiusSm
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                Layout.topMargin: 8
                text: "No connections yet."
                color: Design.ink(theme)
                font.family: Design.serif(theme)
                font.pixelSize: 40
                font.weight: Font.Normal
                font.letterSpacing: -0.4
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
            }

            Text {
                Layout.fillWidth: true
                text: "Add a watch-only connection \u2014 XPub, descriptor, Lightning node, or CSV \u2014 to import transactions."
                color: Design.ink2(theme)
                font.family: Design.sans()
                font.pixelSize: 14
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                lineHeight: 1.35
            }

            ActionButton {
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: 4
                variant: "primary"
                size: "lg"
                text: "+ Add connection"
                enabled: connectionsVM ? connectionsVM.canOpenAddConnection : false
                onClicked: root.requestAddConnection()
            }
        }
    }

    ScrollView {
        anchors.fill: parent
        visible: dashboardVM.hasData
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap

            // Row 1: Balance over time | Connections | Filters | Fiat
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding
                columns: width < 1000 ? (width < 600 ? 1 : 2) : 4
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // Balance over time (placeholder: dimmed chart + overlay)
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    title: "Balance over time"
                    subtitle: "Chart data unavailable yet."

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        implicitHeight: 220
                        color: "transparent"

                        BalanceChart {
                            anchors.fill: parent
                            anchors.margins: 6
                            opacity: 0.35
                            series: [1.0, 1.15, 1.3, 1.45, 1.6, 1.8, 2.0, 2.3, 2.6, 3.0, 3.4, 3.8]
                        }

                        Text {
                            anchors.centerIn: parent
                            text: "DATA UNAVAILABLE"
                            color: Design.ink3(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }
                    }
                }

                // Connections (real data)
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    title: "Connections"
                    subtitle: {
                        var n = connectionsVM ? connectionsVM.connectionCount : 0
                        return n === 0 ? "No wallets yet"
                            : n === 1 ? "One wallet"
                            : n + " wallets"
                    }

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Repeater {
                            model: root.connectionRows

                            delegate: ConnectionRow {
                                Layout.fillWidth: true
                                borderTop: index > 0
                                connectionLabel: modelData["label"] || ""
                                balanceLabel: modelData["balance_label"] || ""
                                statusTone: modelData["status_tone"] || "warn"
                                hideSensitive: root.hideSensitive
                                onClicked: if (connectionsVM) { connectionsVM.selectConnection(modelData["id"] || "") }
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.topMargin: 6
                            Layout.preferredHeight: 1
                            color: Design.line(theme)
                        }

                        ActionButton {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.topMargin: 8
                            Layout.bottomMargin: 4
                            variant: "ghost"
                            size: "sm"
                            text: "+ Add connection"
                            enabled: connectionsVM ? connectionsVM.canOpenAddConnection : false
                            onClicked: root.requestAddConnection()
                        }
                    }
                }

                // Filters (placeholder)
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    title: "Filters"
                    subtitle: "Transaction filters not wired yet."

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        implicitHeight: 220
                        color: "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: "DATA UNAVAILABLE"
                            color: Design.ink3(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }
                    }
                }

                // Fiat · EUR (placeholder)
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    title: "Fiat \u00b7 EUR"
                    subtitle: "Live rates and gains not wired yet."

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        implicitHeight: 220
                        color: "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: "DATA UNAVAILABLE"
                            color: Design.ink3(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }
                    }
                }
            }

            // Row 2: Transactions | Balances
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: width < 1000 ? 1 : 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // Transactions (real table)
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    title: "Transactions"
                    subtitle: {
                        var n = transactionsVM ? transactionsVM.totalCount : root.transactionRows.length
                        return n === 0 ? "No transactions yet"
                            : n === 1 ? "1 entry"
                            : n + " entries"
                    }
                    action: Component {
                        ActionButton {
                            variant: "ghost"
                            size: "sm"
                            text: "Open all \u2192"
                            onClicked: dashboardVM.selectPage("transactions")
                        }
                    }

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        // Column headers
                        RowLayout {
                            Layout.fillWidth: true
                            Layout.leftMargin: theme.spacingSm + 2
                            Layout.rightMargin: theme.spacingSm + 2
                            Layout.topMargin: theme.spacingSm
                            Layout.bottomMargin: theme.spacingSm - 2
                            spacing: theme.spacingSm + 4

                            Text {
                                Layout.preferredWidth: 74
                                text: "DATE"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.2
                            }

                            Text {
                                Layout.preferredWidth: 74
                                text: "TYPE"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.2
                            }

                            Text {
                                Layout.fillWidth: true
                                text: "COUNTERPARTY"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.2
                            }

                            Text {
                                Layout.preferredWidth: 120
                                text: "SATS"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.2
                                horizontalAlignment: Text.AlignRight
                            }

                            Text {
                                Layout.preferredWidth: 90
                                text: "\u20ac"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.2
                                horizontalAlignment: Text.AlignRight
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Design.line(theme)
                        }

                        Repeater {
                            model: root.visibleTransactionRows

                            delegate: Rectangle {
                                Layout.fillWidth: true
                                implicitHeight: 36
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
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    anchors.leftMargin: theme.spacingSm + 2
                                    anchors.rightMargin: theme.spacingSm + 2
                                    spacing: theme.spacingSm + 4

                                    Text {
                                        Layout.preferredWidth: 74
                                        text: (modelData["occurred_at_label"] || "").substring(5, 10)
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                    }

                                    TypeBadge {
                                        Layout.preferredWidth: 74
                                        Layout.alignment: Qt.AlignLeft | Qt.AlignVCenter
                                        label: modelData["type_label"] || modelData["kind_label"] || ""
                                        tone: modelData["type_badge_tone"] || ""
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["counterparty"] || ""
                                        color: Design.ink(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBody
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        Layout.preferredWidth: 120
                                        text: modelData["amount_sats_signed_label"] || ""
                                        color: {
                                            var tone = modelData["type_tone"] || ""
                                            if (tone === "positive") return theme.positive
                                            if (tone === "negative") return Design.accent(theme)
                                            return Design.ink(theme)
                                        }
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                        layer.enabled: root.hideSensitive
                                        layer.effect: FastBlur { radius: 48 }
                                    }

                                    Text {
                                        Layout.preferredWidth: 90
                                        text: modelData["fiat_label"] || ""
                                        color: Design.ink3(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                        elide: Text.ElideRight
                                        layer.enabled: root.hideSensitive
                                        layer.effect: FastBlur { radius: 48 }
                                    }
                                }
                            }
                        }
                    }
                }

                // Balances (placeholder)
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 1
                    title: "Balances"
                    subtitle: "Account-level balances not wired yet."

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        implicitHeight: 340
                        color: "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: "DATA UNAVAILABLE"
                            color: Design.ink3(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }
                    }
                }
            }

            // Row 3: Capital gains | Journal entries | Balance sheet
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                columns: width < 900 ? 1 : 3
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // Capital gains (AT not wired)
                ReportTile {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    title: "Capital gains"
                    subtitle: "AT tax processing not yet available"
                    detail: "\u2014"
                    iconGlyph: "\u2197"
                }

                // Journal entries (real count)
                ReportTile {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    title: "Journal entries"
                    subtitle: "Processed tax journal"
                    detail: {
                        var entries = reportsVM ? reportsVM.summaryCards : []
                        for (var i = 0; i < entries.length; i++) {
                            if ((entries[i]["label"] || "") === "Journal entries") {
                                return (entries[i]["value"] || "0") + " entries"
                            }
                        }
                        return "0 entries"
                    }
                    iconGlyph: "\u2261"
                }

                // Balance sheet (placeholder)
                ReportTile {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    title: "Balance sheet"
                    subtitle: "Assets \u00b7 Liabilities \u00b7 Equity"
                    detail: "Not available yet"
                    iconGlyph: "\u25a4"
                }
            }
        }
    }
}
