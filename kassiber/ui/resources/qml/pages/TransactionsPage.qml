import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    clip: true

    property string searchQuery: ""
    property string activeFilter: "all"
    readonly property int ledgerMinWidth: 1234
    readonly property var transactionTypes: {
        var values = ["all"]
        var source = transactionsVM.items || []
        for (var i = 0; i < source.length; ++i) {
            var label = normalizedType(source[i])
            if (values.indexOf(label) === -1) {
                values.push(label)
            }
        }
        return values
    }
    readonly property var filteredTransactions: {
        var results = []
        var source = transactionsVM.items || []
        var query = String(root.searchQuery || "").toLowerCase()
        for (var i = 0; i < source.length; ++i) {
            var item = source[i]
            var typeName = normalizedType(item)
            if (root.activeFilter !== "all" && typeName !== root.activeFilter) {
                continue
            }
            if (query.length) {
                var haystack = [
                    item["counterparty"] || "",
                    item["wallet"] || "",
                    primaryTag(item),
                    item["tags"] || "",
                    item["description"] || "",
                    item["note"] || "",
                    item["asset"] || ""
                ].join(" ").toLowerCase()
                if (haystack.indexOf(query) === -1) {
                    continue
                }
            }
            results.push(item)
        }
        return results
    }

    function normalizedType(item) {
        var raw = String(item["title"] || item["direction"] || "Entry").trim()
        var lower = raw.toLowerCase()
        if (lower.indexOf("income") !== -1 || lower.indexOf("deposit") !== -1 || lower.indexOf("receive") !== -1 || lower.indexOf("inbound") !== -1 || lower.indexOf("buy") !== -1) {
            return "Income"
        }
        if (lower.indexOf("expense") !== -1 || lower.indexOf("withdraw") !== -1 || lower.indexOf("spend") !== -1 || lower.indexOf("outbound") !== -1 || lower.indexOf("sell") !== -1) {
            return "Expense"
        }
        if (lower.indexOf("transfer") !== -1 || lower.indexOf("move") !== -1) {
            return "Transfer"
        }
        if (lower.indexOf("swap") !== -1 || lower.indexOf("trade") !== -1) {
            return "Swap"
        }
        if (lower.indexOf("consolidation") !== -1) {
            return "Consolidation"
        }
        if (lower.indexOf("rebalance") !== -1) {
            return "Rebalance"
        }
        if (lower.indexOf("mint") !== -1) {
            return "Mint"
        }
        if (lower.indexOf("melt") !== -1) {
            return "Melt"
        }
        if (lower.indexOf("fee") !== -1) {
            return "Fee"
        }
        return raw
    }

    function typeTone(typeName) {
        var key = String(typeName || "").toLowerCase()
        if (key === "income") {
            return theme.ok
        }
        if (key === "expense") {
            return theme.accent
        }
        if (key === "swap") {
            return theme.warn
        }
        if (key === "mint") {
            return theme.pillTeal
        }
        if (key === "melt") {
            return "#A66A3F"
        }
        if (key === "consolidation") {
            return "#5D6B7A"
        }
        if (key === "rebalance") {
            return "#7D6B8A"
        }
        return theme.ink3
    }

    function amountTone(item) {
        var amount = Number(item["amount_msat"] || item["amount"] || 0)
        return amount >= 0 ? theme.ok : theme.ink
    }

    function signedAmountLabel(item) {
        var value = String(item["amount_label"] || "")
        if (!value.length) {
            return "-"
        }
        var amount = Number(item["amount_msat"] || item["amount"] || 0)
        return (amount >= 0 ? "+ " : "- ") + value.replace(/^- /, "").replace(/^\+ /, "")
    }

    function signedFiatLabel(item) {
        var label = String(item["fiat_label"] || "")
        if (!label.length) {
            return "-"
        }
        var value = Number(item["fiat_value"] || 0)
        return (value >= 0 ? "+ " : "- ") + label.replace(/^- /, "").replace(/^\+ /, "")
    }

    function primaryTag(item) {
        var tags = String(item["tags"] || "")
        if (!tags.length || tags === "No tags") {
            return "untagged"
        }
        return tags.split(",")[0].trim()
    }

    function pageYear() {
        var rows = root.filteredTransactions
        if (rows.length) {
            var stamp = String(rows[0]["occurred_at"] || rows[0]["occurred_at_label"] || "")
            if (stamp.length >= 4) {
                return stamp.substring(0, 4)
            }
        }
        return String(new Date().getFullYear())
    }

    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

    Item {
        width: root.availableWidth
        implicitHeight: contentColumn.implicitHeight

        Column {
            id: contentColumn
            width: parent.width
            spacing: 18

            Row {
                width: parent.width
                spacing: 16

                Column {
                    width: Math.max(320, parent.width - actionsRow.width - parent.spacing)
                    spacing: 4

                    Text {
                        text: "Ledger \u00b7 " + root.filteredTransactions.length + " entries \u00b7 " + pageYear()
                        color: theme.ink3
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.capitalization: Font.AllUppercase
                        font.letterSpacing: 1.4
                    }

                    Text {
                        width: parent.width
                        text: "Transactions"
                        color: theme.ink
                        font.family: theme.serifFont
                        font.pixelSize: 32
                        font.weight: Font.Normal
                        font.letterSpacing: -0.4
                        elide: Text.ElideRight
                    }
                }

                Row {
                    id: actionsRow
                    spacing: 8

                    SecondaryButton {
                        text: "Export CSV"
                        size: "sm"
                    }

                    SecondaryButton {
                        text: "Export JSON"
                        size: "sm"
                    }

                    PrimaryButton {
                        text: "Manual entry"
                        size: "sm"
                        minWidth: 108
                    }
                }
            }

            Rectangle {
                width: parent.width
                visible: !transactionsVM.isEmpty
                implicitHeight: filterRow.implicitHeight + 20
                color: theme.paper2
                border.color: theme.line
                border.width: 1

                Row {
                    id: filterRow
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    spacing: 12

                    Row {
                        width: Math.max(280, parent.width - divider.width - filterPills.width - 24)
                        height: 28
                        spacing: 6

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: "\u2315"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 12
                        }

                        TextField {
                            width: parent.width - 20
                            height: 28
                            placeholderText: "Search counterparty, tag, account..."
                            font.family: theme.sansFont
                            font.pixelSize: 12
                            color: theme.ink
                            selectByMouse: true
                            background: Rectangle {
                                color: "transparent"
                                border.width: 0
                            }
                            onTextChanged: root.searchQuery = text
                        }
                    }

                    Rectangle {
                        id: divider
                        width: 1
                        height: 20
                        anchors.verticalCenter: parent.verticalCenter
                        color: theme.line
                    }

                    Flow {
                        id: filterPills
                        width: Math.max(220, contentWidth)
                        spacing: 4

                        Repeater {
                            model: root.transactionTypes

                            Pill {
                                text: modelData
                                active: root.activeFilter === modelData
                                tone: root.activeFilter === modelData ? "ink" : "muted"
                                onClicked: root.activeFilter = modelData
                            }
                        }
                    }
                }
            }

            Card {
                width: parent.width
                visible: transactionsVM.isEmpty
                fillColor: theme.paper2

                Column {
                    width: parent.width
                    spacing: 8

                    Text {
                        text: "No transactions yet"
                        color: theme.ink
                        font.family: theme.serifFont
                        font.pixelSize: 28
                    }

                    Text {
                        width: parent.width
                        text: "Once a wallet syncs or an import lands, this page switches into the Claude-style ledger with local search and type filters."
                        color: theme.ink2
                        font.family: theme.sansFont
                        font.pixelSize: 13
                        wrapMode: Text.WordWrap
                    }
                }
            }

            Card {
                width: parent.width
                visible: !transactionsVM.isEmpty && root.filteredTransactions.length === 0
                fillColor: theme.paper2

                Column {
                    width: parent.width
                    spacing: 8

                    Text {
                        text: "No entries match this filter"
                        color: theme.ink
                        font.family: theme.serifFont
                        font.pixelSize: 28
                    }

                    Text {
                        width: parent.width
                        text: "Try another search query or switch the type pills back to all."
                        color: theme.ink2
                        font.family: theme.sansFont
                        font.pixelSize: 13
                        wrapMode: Text.WordWrap
                    }
                }
            }

            Rectangle {
                width: parent.width
                visible: !transactionsVM.isEmpty && root.filteredTransactions.length > 0
                color: theme.paper2
                border.color: theme.line
                border.width: 1
                implicitHeight: ledgerViewport.implicitHeight

                Item {
                    id: ledgerViewport
                    width: parent.width
                    implicitHeight: Math.min(900, ledgerColumn.implicitHeight)

                    Flickable {
                        anchors.fill: parent
                        clip: true
                        contentWidth: ledgerColumn.width
                        contentHeight: ledgerColumn.implicitHeight
                        interactive: contentWidth > width
                        flickableDirection: Flickable.HorizontalFlick
                        boundsBehavior: Flickable.StopAtBounds

                        Column {
                            id: ledgerColumn
                            width: Math.max(ledgerViewport.width, root.ledgerMinWidth)
                            spacing: 0

                            Rectangle {
                                width: parent.width
                                height: 36
                                color: theme.paper

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 1
                                    color: theme.ink
                                }

                                Row {
                                    anchors.fill: parent
                                    anchors.leftMargin: 14
                                    anchors.rightMargin: 14
                                    spacing: 0

                                    Repeater {
                                        model: [
                                            { "label": "Date / time", "width": 154 },
                                            { "label": "Type", "width": 120 },
                                            { "label": "Wallet", "width": 176 },
                                            { "label": "Asset", "width": 96 },
                                            { "label": "Tag", "width": 134 },
                                            { "label": "Amount", "width": 130, "alignRight": true },
                                            { "label": "Fiat", "width": 126, "alignRight": true },
                                            { "label": "Fee", "width": 104, "alignRight": true },
                                            { "label": "", "width": 12 },
                                            { "label": "Direction", "width": 120 }
                                        ]

                                        Text {
                                            width: modelData["width"]
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            horizontalAlignment: modelData["alignRight"] ? Text.AlignRight : Text.AlignLeft
                                            text: modelData["label"]
                                            color: theme.ink3
                                            font.family: theme.sansFont
                                            font.pixelSize: 9
                                            font.weight: Font.DemiBold
                                            font.capitalization: Font.AllUppercase
                                            font.letterSpacing: 1.2
                                        }
                                    }
                                }
                            }

                            Repeater {
                                model: root.filteredTransactions

                                Rectangle {
                                    width: parent.width
                                    height: 52
                                    color: "transparent"

                                    Rectangle {
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.bottom: parent.bottom
                                        height: 1
                                        color: theme.line
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: transactionsVM.selectTransaction(modelData["id"])
                                    }

                                    Row {
                                        anchors.fill: parent
                                        anchors.leftMargin: 14
                                        anchors.rightMargin: 14
                                        spacing: 0

                                        Text {
                                            width: 154
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            text: modelData["occurred_at_label"] || "-"
                                            color: theme.ink2
                                            font.family: theme.monoFont
                                            font.pixelSize: 11
                                            elide: Text.ElideRight
                                        }

                                        Item {
                                            width: 120
                                            height: parent.height

                                            Rectangle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: Math.min(parent.width, typeText.implicitWidth + 14)
                                                height: 22
                                                radius: 11
                                                color: "transparent"
                                                border.color: root.typeTone(root.normalizedType(modelData))
                                                border.width: 1

                                                Text {
                                                    id: typeText
                                                    anchors.centerIn: parent
                                                    text: root.normalizedType(modelData)
                                                    color: root.typeTone(root.normalizedType(modelData))
                                                    font.family: theme.monoFont
                                                    font.pixelSize: 9
                                                    font.weight: Font.DemiBold
                                                    font.capitalization: Font.AllUppercase
                                                    font.letterSpacing: 0.8
                                                }
                                            }
                                        }

                                        Text {
                                            width: 176
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            text: modelData["wallet"] || "-"
                                            color: theme.ink
                                            font.family: theme.sansFont
                                            font.pixelSize: 12
                                            elide: Text.ElideRight
                                        }

                                        Text {
                                            width: 96
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            text: modelData["asset"] || "-"
                                            color: theme.ink
                                            font.family: theme.monoFont
                                            font.pixelSize: 11
                                            elide: Text.ElideRight
                                        }

                                        Item {
                                            width: 134
                                            height: parent.height

                                            Rectangle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: Math.min(parent.width, tagText.implicitWidth + 14)
                                                height: 22
                                                radius: 2
                                                color: theme.paper
                                                border.color: theme.line
                                                border.width: 1

                                                Text {
                                                    id: tagText
                                                    anchors.centerIn: parent
                                                    text: root.primaryTag(modelData)
                                                    color: theme.ink2
                                                    font.family: theme.monoFont
                                                    font.pixelSize: 10
                                                    font.letterSpacing: 0.4
                                                }
                                            }
                                        }

                                        Text {
                                            width: 130
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            horizontalAlignment: Text.AlignRight
                                            text: root.signedAmountLabel(modelData)
                                            color: root.amountTone(modelData)
                                            font.family: theme.monoFont
                                            font.pixelSize: 11
                                            font.weight: Font.DemiBold
                                        }

                                        Text {
                                            width: 126
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            horizontalAlignment: Text.AlignRight
                                            text: root.signedFiatLabel(modelData)
                                            color: Number(modelData["fiat_value"] || 0) >= 0 ? theme.ok : theme.ink2
                                            font.family: theme.monoFont
                                            font.pixelSize: 11
                                        }

                                        Text {
                                            width: 104
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            horizontalAlignment: Text.AlignRight
                                            text: modelData["fee_label"] || "-"
                                            color: theme.ink3
                                            font.family: theme.monoFont
                                            font.pixelSize: 11
                                        }

                                        Item {
                                            width: 12
                                            height: parent.height
                                        }

                                        Text {
                                            width: 120
                                            height: parent.height
                                            verticalAlignment: Text.AlignVCenter
                                            text: modelData["direction"] || "-"
                                            color: theme.ink3
                                            font.family: theme.sansFont
                                            font.pixelSize: 11
                                            elide: Text.ElideRight
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
