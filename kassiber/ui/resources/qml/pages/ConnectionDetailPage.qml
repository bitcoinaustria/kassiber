import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    signal requestAddConnection()
    clip: true

    readonly property var connection: connectionsVM.selectedItem || ({})
    readonly property var detailRows: connectionsVM.selectedDetails || []
    readonly property var recentTransactions: connectionsVM.selectedTransactions || []
    readonly property var relatedConnections: buildRelatedConnections()
    readonly property real pageWidth: Math.max(0, availableWidth)
    readonly property bool wideLayout: pageWidth >= 1120
    readonly property bool compactHeader: pageWidth < 1080

    function limitItems(items, count) {
        var out = []
        if (!items) {
            return out
        }
        for (var i = 0; i < items.length && i < count; ++i) {
            out.push(items[i])
        }
        return out
    }

    function buildRelatedConnections() {
        var out = []
        var items = connectionsVM.items || []
        var currentId = String(connection["id"] || "")
        for (var i = 0; i < items.length; ++i) {
            if (String(items[i]["id"] || "") !== currentId) {
                out.push(items[i])
            }
        }
        return limitItems(out, 6)
    }

    function valueFor(label, fallback) {
        for (var i = 0; i < detailRows.length; ++i) {
            if (detailRows[i]["label"] === label) {
                return detailRows[i]["value"] || fallback || ""
            }
        }
        return fallback || ""
    }

    function statusColor() {
        return connection["status_tone"] === "ok" ? theme.ok : theme.warn
    }

    function badgeColor(kind) {
        var normalized = String(kind || "").toLowerCase()
        if (normalized.indexOf("descriptor") >= 0) {
            return theme.pillIndigo
        }
        if (normalized.indexOf("xpub") >= 0 || normalized.indexOf("electrum") >= 0) {
            return theme.pillTeal
        }
        if (normalized.indexOf("rpc") >= 0 || normalized.indexOf("esplora") >= 0) {
            return theme.pillOlive
        }
        if (normalized.indexOf("ln") >= 0) {
            return theme.pillAmber
        }
        return theme.pillGreen
    }

    function relatedLabel(item) {
        return (item["label"] || "") + "  \u00b7  " + (item["kind"] || "")
    }

    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

    Item {
        width: root.availableWidth
        implicitHeight: connectionsVM.isEmpty ? emptyState.implicitHeight : contentColumn.implicitHeight

        Column {
            id: contentColumn
            visible: !connectionsVM.isEmpty
            width: parent.width
            spacing: 10

            Item {
                width: parent.width
                implicitHeight: headerGrid.implicitHeight

                GridLayout {
                    id: headerGrid
                    width: parent.width
                    columns: compactHeader ? 1 : 2
                    columnSpacing: 16
                    rowSpacing: 12

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 14

                        SecondaryButton {
                            text: "\u2190"
                            size: "sm"
                            minWidth: 32
                            onClicked: dashboardVM.selectPage("overview")
                        }

                        ProtocolIcon {
                            kind: connection["kind"] || ""
                            size: 40
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 4

                            Text {
                                Layout.fillWidth: true
                                text: (connection["kind"] || "").toUpperCase() + " \u00b7 Connection"
                                color: theme.ink3
                                font.family: theme.monoFont
                                font.pixelSize: 10
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.2
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.fillWidth: true
                                text: connection["label"] || "Connection detail"
                                color: theme.ink
                                font.family: theme.serifFont
                                font.pixelSize: 30
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.fillWidth: true
                                text: connection["subtitle"] || ""
                                color: theme.ink2
                                font.family: theme.sansFont
                                font.pixelSize: 13
                                elide: Text.ElideRight
                            }
                        }
                    }

                    Flow {
                        Layout.fillWidth: true
                        Layout.alignment: compactHeader ? Qt.AlignLeft : Qt.AlignRight | Qt.AlignTop
                        width: compactHeader ? parent.width : Math.max(240, parent.width)
                        spacing: 8

                        SecondaryButton {
                            text: "Sync"
                            size: "sm"
                        }

                        GhostButton {
                            text: "Edit"
                            size: "sm"
                        }

                        DangerButton {
                            text: "Remove"
                            size: "sm"
                        }
                    }
                }
            }

            GridLayout {
                width: parent.width
                columns: pageWidth >= 1260 ? 4 : (pageWidth >= 760 ? 2 : 1)
                columnSpacing: 10
                rowSpacing: 10

                Repeater {
                    model: [
                        {
                            "label": "Balance",
                            "value": connection["balance_label"] || "0 BTC",
                            "sub": connection["subtitle"] || ""
                        },
                        {
                            "label": "Transactions",
                            "value": connection["transaction_count_label"] || "0",
                            "sub": "imported rows"
                        },
                        {
                            "label": "Status",
                            "value": connection["status_label"] || "Needs sync",
                            "sub": connection["chain"] || ""
                        },
                        {
                            "label": "Created",
                            "value": connection["created_at_label"] || "Unknown",
                            "sub": dashboardVM.projectSummary
                        }
                    ]

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 84
                        color: theme.paper2
                        border.color: theme.line
                        border.width: 1

                        Column {
                            anchors.fill: parent
                            anchors.margins: 14
                            spacing: 4

                            Text {
                                text: modelData["label"]
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 10
                                font.bold: true
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.1
                            }

                            Text {
                                text: modelData["value"]
                                color: modelData["label"] === "Status" ? statusColor() : theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 16
                                font.bold: modelData["label"] === "Status"
                            }

                            Text {
                                text: modelData["sub"]
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 11
                            }
                        }
                    }
                }
            }

            GridLayout {
                width: parent.width
                columns: wideLayout ? 2 : 1
                columnSpacing: 10
                rowSpacing: 10

                Card {
                    Layout.fillWidth: true
                    title: "Recent transactions"
                    action: Component {
                        GhostButton {
                            size: "sm"
                            text: "Open all"
                            onClicked: dashboardVM.selectPage("transactions")
                        }
                    }
                    pad: false

                    Column {
                        width: parent.width

                        Rectangle {
                            width: parent.width
                            height: 34
                            color: theme.paper
                            border.width: 0

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
                                        { "label": "Date", "width": 118 },
                                        { "label": "Type", "width": 90 },
                                        { "label": "Amount", "width": 120, "alignRight": true },
                                        { "label": "Fiat", "width": 120, "alignRight": true }
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
                                        font.bold: true
                                        font.capitalization: Font.AllUppercase
                                        font.letterSpacing: 1.2
                                    }
                                }
                            }
                        }

                        Repeater {
                            model: recentTransactions

                            Rectangle {
                                width: parent.width
                                height: 38
                                color: "transparent"
                                border.width: 0

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 1
                                    color: theme.line
                                }

                                Row {
                                    anchors.fill: parent
                                    anchors.leftMargin: 14
                                    anchors.rightMargin: 14
                                    spacing: 0

                                    Text {
                                        width: 118
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["occurred_at_label"] || ""
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 90
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["title"] || ""
                                        color: theme.ink
                                        font.family: theme.monoFont
                                        font.pixelSize: 10
                                        font.capitalization: Font.AllUppercase
                                        font.letterSpacing: 0.8
                                    }

                                    Text {
                                        width: 120
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["amount_sats_signed_label"] || ""
                                        color: Number(modelData["amount_sats"] || 0) >= 0 ? theme.ok : theme.accent
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 120
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["fiat_label"] || ""
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }
                                }
                            }
                        }
                    }
                }

                Column {
                    Layout.fillWidth: true
                    spacing: 10

                    Card {
                        width: parent.width
                        title: "Connection details"

                        GridLayout {
                            width: parent.width
                            columns: 2
                            columnSpacing: 12
                            rowSpacing: 10

                            Repeater {
                                model: limitItems(detailRows, 6)

                                Column {
                                    Layout.fillWidth: true
                                    spacing: 2

                                    Text {
                                        text: modelData["label"]
                                        color: theme.ink3
                                        font.family: theme.sansFont
                                        font.pixelSize: 10
                                        font.bold: true
                                        font.capitalization: Font.AllUppercase
                                        font.letterSpacing: 1.1
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["value"]
                                        color: theme.ink
                                        font.family: theme.monoFont
                                        font.pixelSize: 12
                                        wrapMode: Text.WrapAnywhere
                                    }
                                }
                            }
                        }
                    }

                    Card {
                        width: parent.width
                        title: "Reference"

                        Column {
                            width: parent.width
                            spacing: 10

                            Text {
                                width: parent.width
                                text: connection["reference"] || valueFor("Reference", "No reference available")
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 12
                                wrapMode: Text.WrapAnywhere
                            }

                            Rectangle {
                                width: parent.width
                                height: 1
                                color: theme.line
                            }

                            Text {
                                width: parent.width
                                text: "Backend  \u00b7  " + valueFor("Backend", connection["backend"] || "Local import / none")
                                color: theme.ink2
                                font.family: theme.monoFont
                                font.pixelSize: 11
                                wrapMode: Text.WrapAnywhere
                            }
                        }
                    }

                    Card {
                        width: parent.width
                        title: "Other connections"
                        pad: false

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.relatedConnections

                                Rectangle {
                                    width: parent.width
                                    height: 42
                                    color: "transparent"
                                    border.width: 0

                                    Rectangle {
                                        visible: index > 0
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.top: parent.top
                                        height: 1
                                        color: theme.line
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: connectionsVM.selectConnection(modelData["id"])
                                    }

                                    Row {
                                        anchors.fill: parent
                                        anchors.leftMargin: 14
                                        anchors.rightMargin: 14
                                        spacing: 8

                                        Rectangle {
                                            width: 8
                                            height: 8
                                            radius: 4
                                            anchors.verticalCenter: parent.verticalCenter
                                            color: badgeColor(modelData["kind"])
                                        }

                                        Text {
                                            width: parent.width - 18
                                            anchors.verticalCenter: parent.verticalCenter
                                            text: relatedLabel(modelData)
                                            color: theme.ink2
                                            font.family: theme.monoFont
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

        Column {
            id: emptyState
            visible: connectionsVM.isEmpty
            width: Math.min(parent.width, 620)
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            anchors.topMargin: 110
            spacing: 18

            Text {
                width: parent.width
                text: "No connections yet"
                color: theme.ink
                font.family: theme.serifFont
                font.pixelSize: 36
                horizontalAlignment: Text.AlignHCenter
            }

            Text {
                width: parent.width
                text: "Add a watch-only wallet or import a file first. This screen will then switch into the exported connection-detail layout with live read-only rows."
                color: theme.ink2
                font.family: theme.sansFont
                font.pixelSize: 14
                wrapMode: Text.WordWrap
                lineHeight: 1.5
                horizontalAlignment: Text.AlignHCenter
            }

            PrimaryButton {
                anchors.horizontalCenter: parent.horizontalCenter
                text: connectionsVM.ctaLabel
                onClicked: root.requestAddConnection()
            }
        }
    }
}
