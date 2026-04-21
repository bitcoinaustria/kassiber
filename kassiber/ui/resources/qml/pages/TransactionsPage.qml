import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import Qt5Compat.GraphicalEffects

import "../components"
import "../components/Design.js" as Design

Item {
    id: root

    property bool hideSensitive: false
    property string searchQuery: ""
    property string activeType: "all"

    readonly property var transactionRows: transactionsVM ? transactionsVM.items : []
    readonly property var typeOptions: {
        var options = ["all"]
        var source = transactionsVM ? (transactionsVM.filterOptions || []) : []
        for (var i = 0; i < source.length; i++) {
            options.push(source[i])
        }
        return options
    }
    readonly property var filteredTransactions: {
        var output = []
        var query = root.searchQuery.toLowerCase().trim()
        for (var i = 0; i < root.transactionRows.length; i++) {
            var item = root.transactionRows[i]
            var typeLabel = String(item["type_label"] || item["kind_label"] || "")
            if (root.activeType !== "all" && typeLabel !== root.activeType) {
                continue
            }
            if (!query) {
                output.push(item)
                continue
            }
            var haystack = [
                item["occurred_at_label"] || "",
                item["type_label"] || "",
                item["event_label"] || "",
                item["account_label"] || "",
                item["wallet"] || "",
                item["counterparty"] || "",
                item["description"] || "",
                item["note"] || "",
                item["tag_label"] || "",
                item["asset"] || ""
            ].join(" ").toLowerCase()
            if (haystack.indexOf(query) !== -1) {
                output.push(item)
            }
        }
        return output
    }

    readonly property int colDate: 110
    readonly property int colType: 96
    readonly property int colAccount: 170
    readonly property int colTag: 110
    readonly property int colSats: 130
    readonly property int colFiat: 120

    function amountColor(tone) {
        if (tone === "positive") return theme.positive
        if (tone === "negative") return Design.accent(theme)
        return Design.ink(theme)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: theme.gridGap

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
                    text: "LEDGER  |  "
                        + (transactionsVM ? transactionsVM.totalCount : root.transactionRows.length)
                        + " ENTRIES  |  "
                        + (transactionsVM ? transactionsVM.historyLabel : "LOCAL SNAPSHOT")
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
                text: "Import labels"
                enabled: false
            }

            ActionButton {
                variant: "secondary"
                size: "sm"
                text: "Export labels"
                enabled: false
            }

            Rectangle {
                Layout.preferredWidth: 1
                Layout.preferredHeight: 22
                color: Design.line(theme)
            }

            ActionButton {
                variant: "secondary"
                size: "sm"
                text: "CSV"
                enabled: false
            }

            ActionButton {
                variant: "secondary"
                size: "sm"
                text: "JSON"
                enabled: false
            }

            ActionButton {
                variant: "primary"
                size: "sm"
                text: "Manual entry"
                enabled: false
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.leftMargin: theme.pagePadding
            Layout.rightMargin: theme.pagePadding
            Layout.preferredHeight: filterRow.implicitHeight + theme.spacingSm * 2
            color: Design.paperAlt(theme)
            border.color: Design.line(theme)
            border.width: 1

            RowLayout {
                id: filterRow
                anchors.fill: parent
                anchors.leftMargin: theme.cardPadding - 2
                anchors.rightMargin: theme.cardPadding - 2
                anchors.topMargin: theme.spacingSm
                anchors.bottomMargin: theme.spacingSm
                spacing: theme.spacingSm + 4

                SearchField {
                    id: searchField
                    Layout.fillWidth: true
                    Layout.preferredWidth: 360
                    Layout.maximumWidth: 480
                    text: root.searchQuery
                    placeholderText: "Search counterparty, tag, account..."
                    onTextChanged: root.searchQuery = text
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
                        model: root.typeOptions

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

        ListView {
            id: txList
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.leftMargin: theme.pagePadding
            Layout.rightMargin: theme.pagePadding
            Layout.bottomMargin: theme.pagePadding
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: root.filteredTransactions
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

                    Text { Layout.preferredWidth: root.colDate; text: "DATE"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colType; text: "TYPE"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colAccount; text: "ACCOUNT"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.fillWidth: true; text: "COUNTERPARTY"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colTag; text: "TAG"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                    Text { Layout.preferredWidth: root.colSats; text: "SATS"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                    Text { Layout.preferredWidth: root.colFiat; text: "FIAT"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
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
                        text: modelData["occurred_on_label"] || ""
                        color: Design.ink2(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                    }

                    TypeBadge {
                        Layout.preferredWidth: root.colType
                        Layout.alignment: Qt.AlignLeft | Qt.AlignVCenter
                        label: modelData["type_label"] || modelData["kind_label"] || ""
                        tone: modelData["type_badge_tone"] || "muted"
                    }

                    Text {
                        Layout.preferredWidth: root.colAccount
                        text: modelData["account_label"] || modelData["wallet"] || ""
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.fillWidth: true
                        text: modelData["counterparty"] || ""
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                        elide: Text.ElideRight
                    }

                    TagChip {
                        Layout.preferredWidth: root.colTag
                        Layout.alignment: Qt.AlignVCenter
                        label: modelData["tag_label"] || ""
                    }

                    Text {
                        Layout.preferredWidth: root.colSats
                        text: modelData["amount_sats_signed_label"] || ""
                        color: root.amountColor(modelData["type_tone"] || "")
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                        horizontalAlignment: Text.AlignRight
                        layer.enabled: root.hideSensitive
                        layer.effect: FastBlur { radius: 48 }
                    }

                    Text {
                        Layout.preferredWidth: root.colFiat
                        text: modelData["fiat_label"] || ""
                        color: Design.ink2(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                        horizontalAlignment: Text.AlignRight
                        elide: Text.ElideRight
                        layer.enabled: root.hideSensitive
                        layer.effect: FastBlur { radius: 48 }
                    }
                }
            }

            Rectangle {
                anchors.centerIn: parent
                visible: root.filteredTransactions.length === 0
                width: 280
                height: 48
                color: "transparent"

                Text {
                    anchors.centerIn: parent
                    text: root.transactionRows.length === 0
                        ? "No transactions loaded yet."
                        : "No transactions match these filters."
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontCaption
                    font.weight: Font.DemiBold
                    font.letterSpacing: 1.2
                }
            }
        }
    }
}
