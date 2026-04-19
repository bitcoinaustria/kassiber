import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    signal requestAddConnection()
    clip: true

    readonly property var connections: connectionsVM.items || []
    readonly property var transactions: transactionsVM.items || []
    readonly property var metrics: dashboardVM.overviewMetrics || []
    readonly property var highlights: dashboardVM.overviewHighlights || []
    readonly property var reportTiles: reportsVM.items || []
    readonly property real pageWidth: Math.max(availableWidth, 980)
    readonly property bool wideLayout: pageWidth >= 1260
    readonly property bool midLayout: pageWidth >= 1080
    readonly property var balanceSeries: buildBalanceSeries()

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

    function metricByLabel(label) {
        for (var i = 0; i < metrics.length; ++i) {
            if (metrics[i]["label"] === label) {
                return metrics[i]
            }
        }
        return {}
    }

    function highlightByTitle(title) {
        for (var i = 0; i < highlights.length; ++i) {
            if (highlights[i]["title"] === title) {
                return highlights[i]
            }
        }
        return {}
    }

    function totalBalanceMsat() {
        var total = 0
        for (var i = 0; i < connections.length; ++i) {
            total += Number(connections[i]["balance_msat"] || 0)
        }
        return total
    }

    function totalBalanceBtc() {
        return (totalBalanceMsat() / 100000000000.0).toFixed(8)
    }

    function totalTransactionFiat() {
        var total = 0
        for (var i = 0; i < transactions.length; ++i) {
            total += Math.abs(Number(transactions[i]["fiat_value"] || 0))
        }
        return total
    }

    function currencyLabel() {
        var profile = String((highlightByTitle("Profile")["value"] || "EUR").split("|")[0] || "EUR")
        return profile.trim() || "EUR"
    }

    function totalFiatLabel() {
        return currencyLabel() + " " + Number(totalTransactionFiat()).toLocaleString(Qt.locale("de_AT"), "f", 2)
    }

    function reportStatus() {
        return reportsVM.statusTitle || "Needs journals"
    }

    function reportStatusBody() {
        return reportsVM.statusBody || ""
    }

    function buildBalanceSeries() {
        var samples = []
        var running = 0
        for (var i = transactions.length - 1; i >= 0; --i) {
            running += Number(transactions[i]["amount_msat"] || transactions[i]["amount"] || 0) / 100000000000.0
            if (running < 0) {
                running = 0
            }
            samples.push(running)
        }
        if (!samples.length) {
            return [0.08, 0.11, 0.13, 0.16, 0.18, 0.24, 0.28, 0.32, 0.34, 0.39, 0.44, 0.48]
        }
        if (samples.length === 1) {
            return [
                samples[0] * 0.35, samples[0] * 0.4, samples[0] * 0.45, samples[0] * 0.5,
                samples[0] * 0.58, samples[0] * 0.65, samples[0] * 0.72, samples[0] * 0.78,
                samples[0] * 0.84, samples[0] * 0.9, samples[0] * 0.95, samples[0]
            ]
        }

        var out = []
        var buckets = 12
        for (var bucket = 0; bucket < buckets; ++bucket) {
            var position = bucket * (samples.length - 1) / (buckets - 1)
            var lower = Math.floor(position)
            var upper = Math.min(samples.length - 1, Math.ceil(position))
            var fraction = position - lower
            out.push(samples[lower] * (1 - fraction) + samples[upper] * fraction)
        }
        return out
    }

    function formatSignedFiat(value) {
        var numeric = Number(value || 0)
        var amount = Qt.locale("de_AT").toString(Math.abs(numeric), "f", 2)
        if (numeric > 0) {
            return "+ " + amount
        }
        if (numeric < 0) {
            return "- " + amount
        }
        return amount
    }

    function typeColor(tone) {
        if (tone === "ok" || tone === "income") {
            return theme.ok
        }
        if (tone === "warn" || tone === "expense") {
            return theme.accent
        }
        return theme.ink2
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

    function balanceRows() {
        return [
            {
                "label": "Assets",
                "sub": "Resources owned",
                "value": totalBalanceBtc() + " BTC",
                "children": limitItems(connections, 4)
            },
            {
                "label": "Transactions",
                "sub": "Imported history",
                "value": (metricByLabel("Transactions")["value"] || "0") + " rows",
                "children": []
            },
            {
                "label": "Journal entries",
                "sub": "Processed lots",
                "value": metricByLabel("Journal entries")["value"] || "0",
                "children": []
            },
            {
                "label": "Quarantines",
                "sub": "Needs review",
                "value": metricByLabel("Quarantines")["value"] || "0",
                "children": []
            }
        ]
    }

    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

    Item {
        width: root.availableWidth
        implicitHeight: dashboardVM.hasData ? contentColumn.implicitHeight : emptyShell.implicitHeight

        Column {
            id: contentColumn
            visible: dashboardVM.hasData
            width: parent.width
            spacing: 10

            GridLayout {
                width: parent.width
                columns: wideLayout ? 12 : (midLayout ? 6 : 1)
                columnSpacing: 10
                rowSpacing: 10

                Card {
                    Layout.fillWidth: true
                    Layout.columnSpan: wideLayout ? 4 : (midLayout ? 6 : 1)
                    eyebrow: "Overview"
                    title: "Balance over time"
                    action: Component {
                        Row {
                            spacing: 4

                            Pill {
                                text: "BTC"
                                active: true
                            }

                            Pill {
                                text: currencyLabel()
                                active: false
                                tone: "muted"
                            }
                        }
                    }

                    Column {
                        width: parent.width
                        spacing: 8

                        Canvas {
                            id: chartCanvas
                            width: parent.width
                            height: 180
                            property var points: root.balanceSeries

                            onPointsChanged: requestPaint()

                            onPaint: {
                                var ctx = getContext("2d")
                                ctx.reset()

                                var padLeft = 42
                                var padRight = 12
                                var padTop = 12
                                var padBottom = 22
                                var plotWidth = width - padLeft - padRight
                                var plotHeight = height - padTop - padBottom
                                var maxValue = 0
                                var i

                                for (i = 0; i < points.length; ++i) {
                                    if (points[i] > maxValue) {
                                        maxValue = points[i]
                                    }
                                }
                                if (maxValue <= 0) {
                                    maxValue = 1
                                }
                                maxValue *= 1.15

                                ctx.strokeStyle = theme.line
                                ctx.lineWidth = 1
                                ctx.setLineDash([2, 3])
                                for (i = 0; i < 5; ++i) {
                                    var y = padTop + (plotHeight * i / 4)
                                    ctx.beginPath()
                                    ctx.moveTo(padLeft, y)
                                    ctx.lineTo(width - padRight, y)
                                    ctx.stroke()
                                }

                                ctx.setLineDash([])
                                if (points.length < 2) {
                                    return
                                }

                                function pointX(index) {
                                    return padLeft + plotWidth * index / (points.length - 1)
                                }

                                function pointY(value) {
                                    return padTop + plotHeight - (value / maxValue) * plotHeight
                                }

                                ctx.beginPath()
                                for (i = 0; i < points.length; ++i) {
                                    var x = pointX(i)
                                    var y2 = pointY(points[i])
                                    if (i === 0) {
                                        ctx.moveTo(x, y2)
                                    } else {
                                        ctx.lineTo(x, y2)
                                    }
                                }

                                ctx.lineTo(pointX(points.length - 1), height - padBottom)
                                ctx.lineTo(pointX(0), height - padBottom)
                                ctx.closePath()
                                ctx.fillStyle = Qt.rgba(0.54, 0.12, 0.17, 0.08)
                                ctx.fill()

                                ctx.beginPath()
                                for (i = 0; i < points.length; ++i) {
                                    var x2 = pointX(i)
                                    var y3 = pointY(points[i])
                                    if (i === 0) {
                                        ctx.moveTo(x2, y3)
                                    } else {
                                        ctx.lineTo(x2, y3)
                                    }
                                }
                                ctx.strokeStyle = theme.accent
                                ctx.lineWidth = 1.5
                                ctx.stroke()

                                ctx.fillStyle = theme.paper2
                                ctx.strokeStyle = theme.accent
                                ctx.lineWidth = 1
                                for (i = 0; i < points.length; ++i) {
                                    var cx = pointX(i)
                                    var cy = pointY(points[i])
                                    ctx.beginPath()
                                    ctx.arc(cx, cy, 2.5, 0, Math.PI * 2)
                                    ctx.fill()
                                    ctx.stroke()
                                }
                            }
                        }

                        Row {
                            spacing: 22

                            Column {
                                spacing: 2

                                Text {
                                    text: "Current"
                                    color: theme.ink3
                                    font.family: theme.sansFont
                                    font.pixelSize: 10
                                    font.bold: true
                                    font.capitalization: Font.AllUppercase
                                    font.letterSpacing: 1.1
                                }

                                Text {
                                    text: totalBalanceBtc() + " BTC"
                                    color: theme.ink
                                    font.family: theme.monoFont
                                    font.pixelSize: 13
                                }
                            }

                            Column {
                                spacing: 2

                                Text {
                                    text: "Delta YTD"
                                    color: theme.ink3
                                    font.family: theme.sansFont
                                    font.pixelSize: 10
                                    font.bold: true
                                    font.capitalization: Font.AllUppercase
                                    font.letterSpacing: 1.1
                                }

                                Text {
                                    text: "+" + (balanceSeries.length ? balanceSeries[balanceSeries.length - 1] - balanceSeries[0] : 0).toFixed(8) + " BTC"
                                    color: theme.ok
                                    font.family: theme.monoFont
                                    font.pixelSize: 13
                                }
                            }

                            Column {
                                spacing: 2

                                Text {
                                    text: "Delta 30D"
                                    color: theme.ink3
                                    font.family: theme.sansFont
                                    font.pixelSize: 10
                                    font.bold: true
                                    font.capitalization: Font.AllUppercase
                                    font.letterSpacing: 1.1
                                }

                                Text {
                                    text: (balanceSeries.length > 2 ? balanceSeries[balanceSeries.length - 1] - balanceSeries[Math.max(0, balanceSeries.length - 3)] : 0).toFixed(8) + " BTC"
                                    color: theme.accent
                                    font.family: theme.monoFont
                                    font.pixelSize: 13
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    Layout.columnSpan: wideLayout ? 3 : (midLayout ? 3 : 1)
                    title: "Connections"
                    action: Component {
                        Text {
                            text: connections.length
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 10
                        }
                    }
                    pad: false

                    Column {
                        width: parent.width

                        Repeater {
                            model: root.connections

                            Rectangle {
                                width: parent.width
                                height: 62
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
                                    hoverEnabled: true
                                    onClicked: {
                                        connectionsVM.selectConnection(modelData["id"])
                                        dashboardVM.selectPage("connection-detail")
                                    }
                                }

                                Row {
                                    anchors.fill: parent
                                    anchors.leftMargin: 14
                                    anchors.rightMargin: 14
                                    spacing: 10

                                    Rectangle {
                                        width: 24
                                        height: 24
                                        anchors.verticalCenter: parent.verticalCenter
                                        color: badgeColor(modelData["kind"])
                                        radius: 2

                                        Text {
                                            anchors.centerIn: parent
                                            text: String(modelData["kind"] || "").substring(0, 2).toUpperCase()
                                            color: theme.paper2
                                            font.family: theme.monoFont
                                            font.pixelSize: 9
                                            font.bold: true
                                        }
                                    }

                                    Column {
                                        width: parent.width - 70
                                        anchors.verticalCenter: parent.verticalCenter
                                        spacing: 2

                                        Text {
                                            width: parent.width
                                            text: modelData["label"]
                                            color: theme.ink
                                            font.family: theme.sansFont
                                            font.pixelSize: 12
                                            font.bold: true
                                            elide: Text.ElideRight
                                        }

                                        Text {
                                            width: parent.width
                                            text: (modelData["kind"] || "") + "  |  " + (modelData["transaction_count_label"] || "0")
                                            color: theme.ink3
                                            font.family: theme.monoFont
                                            font.pixelSize: 10
                                            elide: Text.ElideRight
                                        }
                                    }

                                    Rectangle {
                                        width: 7
                                        height: 7
                                        radius: 4
                                        anchors.verticalCenter: parent.verticalCenter
                                        color: modelData["status_tone"] === "ok" ? theme.ok : theme.warn
                                    }
                                }
                            }
                        }

                        Rectangle {
                            width: parent.width
                            height: 48
                            color: theme.paper
                            border.width: 0

                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                height: 1
                                color: theme.line
                            }

                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.requestAddConnection()
                            }

                            Row {
                                anchors.fill: parent
                                anchors.leftMargin: 14
                                anchors.rightMargin: 14
                                spacing: 8

                                Rectangle {
                                    width: 24
                                    height: 24
                                    anchors.verticalCenter: parent.verticalCenter
                                    color: "transparent"
                                    border.color: theme.ink2
                                    border.width: 1
                                    radius: 2

                                    Text {
                                        anchors.centerIn: parent
                                        text: "+"
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 12
                                    }
                                }

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: "Add connection"
                                    color: theme.ink
                                    font.family: theme.sansFont
                                    font.pixelSize: 12
                                    font.bold: true
                                }

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: "watch-only"
                                    color: theme.ink3
                                    font.family: theme.monoFont
                                    font.pixelSize: 10
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    Layout.columnSpan: wideLayout ? 2 : (midLayout ? 3 : 1)
                    title: "Filters"
                    action: Component {
                        Text {
                            text: "RESET"
                            color: theme.ink2
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.letterSpacing: 1.0
                        }
                    }

                    Column {
                        width: parent.width
                        spacing: 10

                        Row {
                            spacing: 6

                            InputField {
                                width: (parent.width - 6) / 2
                                label: "From"
                                mono: true
                                text: "2026-01-01"
                                readOnly: true
                            }

                            InputField {
                                width: (parent.width - 6) / 2
                                label: "To"
                                mono: true
                                text: "2026-04-19"
                                readOnly: true
                            }
                        }

                        Flow {
                            width: parent.width
                            spacing: 4

                            Repeater {
                                model: ["1W", "MTD", "1M", "QTD", "LQ", "YTD"]

                                Pill {
                                    text: modelData
                                    active: modelData === "YTD"
                                    tone: modelData === "YTD" ? "ink" : "muted"
                                }
                            }
                        }

                        InputField {
                            width: parent.width
                            label: "Account"
                            text: "All accounts"
                            rightText: "v"
                            readOnly: true
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    Layout.columnSpan: wideLayout ? 3 : (midLayout ? 6 : 1)
                    title: "Profile \u00b7 " + currencyLabel()

                    GridLayout {
                        width: parent.width
                        columns: 2
                        columnSpacing: 10
                        rowSpacing: 10

                        Item {
                            Layout.columnSpan: 2
                            implicitHeight: 54

                            Column {
                                spacing: 2

                                Text {
                                    text: totalFiatLabel()
                                    color: theme.ink
                                    font.family: theme.serifFont
                                    font.pixelSize: 28
                                }

                                Text {
                                    text: reportStatus()
                                    color: reportsVM.statusTone === "ok" ? theme.ok : theme.warn
                                    font.family: theme.monoFont
                                    font.pixelSize: 10
                                }
                            }
                        }

                        Column {
                            spacing: 2

                            Text {
                                text: "Connections"
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 10
                                font.bold: true
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.1
                            }

                            Text {
                                text: metricByLabel("Connections")["value"] || "0"
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 13
                            }
                        }

                        Column {
                            spacing: 2

                            Text {
                                text: "Transactions"
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 10
                                font.bold: true
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.1
                            }

                            Text {
                                text: metricByLabel("Transactions")["value"] || "0"
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 13
                            }
                        }

                        Column {
                            spacing: 2

                            Text {
                                text: "Report state"
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 10
                                font.bold: true
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.1
                            }

                            Text {
                                text: highlightByTitle("Report readiness")["value"] || reportStatus()
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 13
                            }
                        }

                        Column {
                            spacing: 2

                            Text {
                                text: "Latest activity"
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 10
                                font.bold: true
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.1
                            }

                            Text {
                                text: highlightByTitle("Latest activity")["value"] || "No imported data"
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 13
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    Layout.columnSpan: wideLayout ? 7 : (midLayout ? 6 : 1)
                    title: "Transactions"
                    action: Component {
                        Text {
                            text: "OPEN ALL \u2192"
                            color: theme.ink
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.letterSpacing: 1.0
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

                            Row {
                                anchors.fill: parent
                                anchors.leftMargin: 14
                                anchors.rightMargin: 14
                                spacing: 0

                                Repeater {
                                    model: [
                                        { "label": "Date", "width": 98 },
                                        { "label": "Type", "width": 84 },
                                        { "label": "Counterparty", "width": 0, "fill": true },
                                        { "label": "Sats", "width": 110, "alignRight": true },
                                        { "label": currencyLabel(), "width": 110, "alignRight": true }
                                    ]

                                    Text {
                                        width: modelData["fill"] ? parent.width - 98 - 84 - 110 - 110 : modelData["width"]
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["label"]
                                        color: theme.ink3
                                        font.family: theme.sansFont
                                        font.pixelSize: 9
                                        font.bold: true
                                        font.capitalization: Font.AllUppercase
                                        font.letterSpacing: 1.2
                                        horizontalAlignment: modelData["alignRight"] ? Text.AlignRight : Text.AlignLeft
                                    }
                                }
                            }

                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                height: 1
                                color: theme.ink
                            }
                        }

                        Repeater {
                            model: root.limitItems(root.transactions, 6)

                            Rectangle {
                                width: parent.width
                                height: 40
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
                                        width: 98
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: String(modelData["occurred_at_label"] || "").replace(" UTC", "")
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 84
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["kind_label"] || modelData["title"] || ""
                                        color: typeColor(modelData["type_tone"])
                                        font.family: theme.monoFont
                                        font.pixelSize: 10
                                        font.capitalization: Font.AllUppercase
                                        font.letterSpacing: 0.8
                                    }

                                    Text {
                                        width: parent.width - 98 - 84 - 110 - 110
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["counterparty"] || modelData["wallet"] || ""
                                        color: theme.ink
                                        font.family: theme.sansFont
                                        font.pixelSize: 12
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        width: 110
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["amount_sats_signed_label"] || ""
                                        color: Number(modelData["amount_sats"] || 0) >= 0 ? theme.ok : theme.accent
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 110
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: formatSignedFiat(modelData["fiat_value"])
                                        color: Number(modelData["fiat_value"] || 0) >= 0 ? theme.ok : theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    Layout.columnSpan: wideLayout ? 5 : (midLayout ? 6 : 1)
                    title: "Balances"

                    Column {
                        width: parent.width
                        spacing: 0

                        Repeater {
                            model: root.balanceRows()

                            Column {
                                width: parent.width

                                Rectangle {
                                    width: parent.width
                                    height: 40
                                    color: "transparent"
                                    border.width: 0

                                    Rectangle {
                                        visible: index < root.balanceRows().length - 1 || (modelData["children"] && modelData["children"].length > 0)
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.bottom: parent.bottom
                                        height: 1
                                        color: theme.line
                                    }

                                    Row {
                                        anchors.fill: parent
                                        spacing: 10

                                        Column {
                                            width: parent.width - 160
                                            anchors.verticalCenter: parent.verticalCenter
                                            spacing: 1

                                            Text {
                                                text: modelData["label"]
                                                color: theme.ink
                                                font.family: theme.serifFont
                                                font.pixelSize: 15
                                            }

                                            Text {
                                                text: modelData["sub"]
                                                color: theme.ink3
                                                font.family: theme.sansFont
                                                font.pixelSize: 11
                                            }
                                        }

                                        Text {
                                            width: 150
                                            anchors.verticalCenter: parent.verticalCenter
                                            horizontalAlignment: Text.AlignRight
                                            text: modelData["value"]
                                            color: modelData["label"] === "Quarantines" ? theme.accent : theme.ink
                                            font.family: theme.monoFont
                                            font.pixelSize: 12
                                        }
                                    }
                                }

                                Repeater {
                                    model: modelData["children"] || []

                                    Rectangle {
                                        width: parent.width
                                        height: 30
                                        color: "transparent"
                                        border.width: 0

                                        Rectangle {
                                            anchors.left: parent.left
                                            anchors.right: parent.right
                                            anchors.bottom: parent.bottom
                                            height: 1
                                            color: theme.line
                                            opacity: 0.5
                                        }

                                        Row {
                                            anchors.fill: parent
                                            anchors.leftMargin: 18
                                            spacing: 8

                                            Text {
                                                width: parent.width - 148
                                                anchors.verticalCenter: parent.verticalCenter
                                                text: "\u2192 " + (modelData["label"] || "")
                                                color: theme.ink2
                                                font.family: theme.sansFont
                                                font.pixelSize: 11
                                                elide: Text.ElideRight
                                            }

                                            Text {
                                                width: 140
                                                anchors.verticalCenter: parent.verticalCenter
                                                horizontalAlignment: Text.AlignRight
                                                text: modelData["balance_label"] || modelData["balance_short"] || ""
                                                color: theme.ink2
                                                font.family: theme.monoFont
                                                font.pixelSize: 11
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            GridLayout {
                width: parent.width
                columns: wideLayout ? 3 : 1
                columnSpacing: 10
                rowSpacing: 10

                Repeater {
                    model: root.limitItems(root.reportTiles, 3)

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.columnSpan: 1
                        implicitHeight: 92
                        color: theme.paper2
                        border.color: theme.line
                        border.width: 1

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: dashboardVM.selectPage("reports")
                        }

                        Row {
                            anchors.fill: parent
                            anchors.margins: 16
                            spacing: 14

                            Rectangle {
                                width: 34
                                height: 34
                                color: "transparent"
                                border.color: theme.ink
                                border.width: 1

                                Text {
                                    anchors.centerIn: parent
                                    text: index === 0 ? "\u2197" : (index === 1 ? "\u2261" : "\u25ad")
                                    color: theme.ink
                                    font.family: theme.serifFont
                                    font.pixelSize: 18
                                }
                            }

                            Column {
                                width: parent.width - 60
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 3

                                Text {
                                    width: parent.width
                                    text: modelData["label"] || ""
                                    color: theme.ink
                                    font.family: theme.serifFont
                                    font.pixelSize: 18
                                    elide: Text.ElideRight
                                }

                                Text {
                                    width: parent.width
                                    text: modelData["summary"] || ""
                                    color: theme.ink3
                                    font.family: theme.sansFont
                                    font.pixelSize: 11
                                    elide: Text.ElideRight
                                }

                                Text {
                                    width: parent.width
                                    text: modelData["status"] || ""
                                    color: modelData["status_tone"] === "ok" ? theme.ok : theme.warn
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

        Column {
            id: emptyShell
            visible: !dashboardVM.hasData
            width: Math.min(parent.width, 620)
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            anchors.topMargin: 110
            spacing: 20

            Grid {
                anchors.horizontalCenter: parent.horizontalCenter
                columns: 4
                columnSpacing: 6
                rowSpacing: 6

                Repeater {
                    model: 12

                    Rectangle {
                        width: 40
                        height: 28
                        color: "transparent"
                        border.color: theme.line2
                        border.width: 1
                        opacity: 0.45
                    }
                }
            }

            Text {
                width: parent.width
                text: dashboardVM.shellTitle
                color: theme.ink
                font.family: theme.serifFont
                font.pixelSize: 36
                horizontalAlignment: Text.AlignHCenter
            }

            Text {
                width: parent.width
                text: dashboardVM.shellBody
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
