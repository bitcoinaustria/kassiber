import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    clip: true

    property int selectedYear: 2025
    property string selectedMethod: "fifo"
    property int previewStep: 1
    readonly property real pageWidth: Math.max(availableWidth, 980)
    readonly property bool stackedLayout: pageWidth < 1080
    readonly property real leftRailWidth: stackedLayout ? pageWidth : 304
    readonly property real rightColumnWidth: stackedLayout ? pageWidth : pageWidth - leftRailWidth - 14
    readonly property var yearOptions: [2023, 2024, 2025, 2026]
    readonly property var methodOptions: reportsVM.methodOptions || []
    readonly property var policyRows: reportsVM.policyRows || []
    readonly property var summaryCards: reportsVM.summaryCards || []
    readonly property var lotRows: reportsVM.previewRows || []
    readonly property var exportFormats: reportsVM.exportFormats || []

    function toneColor(tone) {
        if (tone === "ok") {
            return theme.ok
        }
        if (tone === "warn") {
            return theme.accent
        }
        return theme.ink
    }

    function summaryValue(label, fallback) {
        for (var i = 0; i < summaryCards.length; ++i) {
            if (summaryCards[i]["label"] === label) {
                return summaryCards[i]["value"] || fallback || ""
            }
        }
        return fallback || ""
    }

    function totalSats() {
        var total = 0
        for (var i = 0; i < lotRows.length; ++i) {
            total += Number(String(lotRows[i]["sats"] || "0").replace(/,/g, ""))
        }
        return total.toLocaleString(Qt.locale("en_US"))
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
                spacing: 18

                Column {
                    width: Math.max(320, parent.width - headerActions.width - parent.spacing)
                    spacing: 4

                    Text {
                        text: "Report \u00b7 \u00a727a EStG \u00b7 Austria"
                        color: theme.ink3
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.capitalization: Font.AllUppercase
                        font.letterSpacing: 1.4
                    }

                    Text {
                        width: parent.width
                        text: "Capital gains"
                        color: theme.ink
                        font.family: theme.serifFont
                        font.pixelSize: 32
                        font.weight: Font.Normal
                        font.letterSpacing: -0.4
                        elide: Text.ElideRight
                    }
                }

                Row {
                    id: headerActions
                    spacing: 18

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "STEP " + root.previewStep + " / 2"
                        color: theme.ink3
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.capitalization: Font.AllUppercase
                        font.letterSpacing: 1.2
                    }

                    Rectangle {
                        implicitWidth: journalsLabel.implicitWidth + 20
                        implicitHeight: 28
                        color: "transparent"
                        border.color: toneColor(reportsVM.statusTone)
                        border.width: 1
                        radius: 14

                        Text {
                            id: journalsLabel
                            anchors.centerIn: parent
                            text: reportsVM.statusTone === "ok" ? "Ready" : "Needs journals"
                            color: toneColor(reportsVM.statusTone)
                            font.family: theme.sansFont
                            font.pixelSize: 11
                            font.bold: true
                        }
                    }

                    GhostButton {
                        text: "Back"
                        size: "sm"
                        onClicked: dashboardVM.selectPage("overview")
                    }
                }
            }

            Rectangle {
                width: parent.width
                implicitHeight: noticeText.implicitHeight + 18
                color: theme.paper2
                border.color: theme.line
                border.width: 1

                Text {
                    id: noticeText
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14
                    text: reportsVM.statusBody
                    color: theme.ink2
                    font.family: theme.sansFont
                    font.pixelSize: 12
                    wrapMode: Text.WordWrap
                }
            }

            Item {
                width: parent.width
                implicitHeight: stackedLayout ? stackedBody.implicitHeight : wideBody.implicitHeight

                Column {
                    id: stackedBody
                    visible: stackedLayout
                    width: parent.width
                    spacing: 14

                    Column {
                        width: parent.width
                        spacing: 10

                        ReportRail {
                            width: parent.width
                        }

                        ReportPreview {
                            width: parent.width
                        }
                    }
                }

                Row {
                    id: wideBody
                    visible: !stackedLayout
                    width: parent.width
                    spacing: 14

                    ReportRail {
                        width: root.leftRailWidth
                    }

                    ReportPreview {
                        width: root.rightColumnWidth
                    }
                }
            }
        }
    }

    component ReportRail: Column {
        spacing: 10

        Card {
            width: parent.width
            title: "Reporting period"

            Column {
                width: parent.width
                spacing: 10

                Flow {
                    width: parent.width
                    spacing: 6

                    Repeater {
                        model: root.yearOptions

                        Pill {
                            text: String(modelData)
                            active: root.selectedYear === modelData
                            tone: root.selectedYear === modelData ? "ink" : "muted"
                            onClicked: root.selectedYear = modelData
                        }
                    }
                }

                Row {
                    width: parent.width
                    spacing: 8

                    InputField {
                        width: (parent.width - 8) / 2
                        label: "From"
                        mono: true
                        text: root.selectedYear + "-01-01"
                        readOnly: true
                    }

                    InputField {
                        width: (parent.width - 8) / 2
                        label: "To"
                        mono: true
                        text: root.selectedYear + "-12-31"
                        readOnly: true
                    }
                }
            }
        }

        Card {
            width: parent.width
            title: "Cost-basis method"

            Column {
                width: parent.width
                spacing: 6

                Repeater {
                    model: root.methodOptions

                    Rectangle {
                        width: parent.width
                        implicitHeight: 50
                        color: root.selectedMethod === modelData["id"] ? theme.paper : "transparent"
                        border.color: theme.line
                        border.width: 1

                        MouseArea {
                            anchors.fill: parent
                            onClicked: root.selectedMethod = modelData["id"]
                        }

                        Row {
                            anchors.fill: parent
                            anchors.leftMargin: 10
                            anchors.rightMargin: 10
                            spacing: 10

                            Rectangle {
                                width: 16
                                height: 16
                                radius: 8
                                anchors.verticalCenter: parent.verticalCenter
                                color: "transparent"
                                border.color: root.selectedMethod === modelData["id"] ? theme.accent : theme.line
                                border.width: 1

                                Rectangle {
                                    anchors.centerIn: parent
                                    width: 8
                                    height: 8
                                    radius: 4
                                    visible: root.selectedMethod === modelData["id"]
                                    color: theme.accent
                                }
                            }

                            Column {
                                width: parent.width - 32
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 2

                                Text {
                                    width: parent.width
                                    text: modelData["label"] || ""
                                    color: theme.ink
                                    font.family: theme.monoFont
                                    font.pixelSize: 12
                                    font.bold: true
                                    elide: Text.ElideRight
                                }

                                Text {
                                    width: parent.width
                                    text: modelData["detail"] || ""
                                    color: theme.ink3
                                    font.family: theme.sansFont
                                    font.pixelSize: 11
                                    wrapMode: Text.WordWrap
                                }
                            }
                        }
                    }
                }
            }
        }

        Card {
            width: parent.width
            title: "Policy"

            Column {
                width: parent.width
                spacing: 8

                Repeater {
                    model: root.policyRows

                    Row {
                        width: parent.width
                        spacing: 10

                        Rectangle {
                            width: 30
                            height: 16
                            anchors.verticalCenter: parent.verticalCenter
                            color: modelData["enabled"] ? theme.ink : theme.line2

                            Rectangle {
                                x: modelData["enabled"] ? 16 : 2
                                y: 2
                                width: 12
                                height: 12
                                color: theme.paper2
                            }
                        }

                        Text {
                            width: parent.width - 40
                            anchors.verticalCenter: parent.verticalCenter
                            text: modelData["label"] || ""
                            color: theme.ink2
                            font.family: theme.sansFont
                            font.pixelSize: 12
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }
        }

        PrimaryButton {
            width: parent.width
            text: root.previewStep === 1 ? "Generate preview" : "Preview ready"
            enabled: reportsVM.items.length > 0
            onClicked: root.previewStep = 2
        }

        Text {
            width: parent.width
            text: "These controls only reshape the desktop preview today. Report generation and export formats still depend on CLI-backed actions."
            color: theme.ink3
            font.family: theme.sansFont
            font.pixelSize: 11
            wrapMode: Text.WordWrap
            lineHeight: 1.45
        }
    }

    component ReportPreview: Column {
        spacing: 10

        Grid {
            width: parent.width
            columns: root.stackedLayout ? 2 : 4
            columnSpacing: 10
            rowSpacing: 10

            Repeater {
                model: root.summaryCards

                Rectangle {
                    width: root.stackedLayout ? (parent.width - parent.columnSpacing) / 2 : (parent.width - 30) / 4
                    height: 88
                    color: theme.paper2
                    border.color: theme.line
                    border.width: 1

                    Column {
                        anchors.fill: parent
                        anchors.margins: 14
                        spacing: 4

                        Text {
                            text: modelData["label"] || ""
                            color: theme.ink3
                            font.family: theme.sansFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.capitalization: Font.AllUppercase
                            font.letterSpacing: 1.1
                        }

                        Text {
                            text: modelData["value"] || ""
                            color: toneColor(modelData["tone"])
                            font.family: theme.serifFont
                            font.pixelSize: 24
                            font.weight: Font.Normal
                        }

                        Text {
                            text: modelData["detail"] || ""
                            color: theme.ink3
                            font.family: theme.sansFont
                            font.pixelSize: 11
                        }
                    }
                }
            }
        }

        Card {
            width: parent.width
            title: "Disposed lots \u00b7 " + root.selectedYear
            pad: false

            Item {
                width: parent.width
                implicitHeight: 36 + (root.lotRows.length * 38) + 40

                Flickable {
                    anchors.fill: parent
                    clip: true
                    contentWidth: Math.max(parent.width, 776)
                    contentHeight: lotsColumn.implicitHeight
                    interactive: contentWidth > width
                    flickableDirection: Flickable.HorizontalFlick
                    boundsBehavior: Flickable.StopAtBounds

                    Column {
                        id: lotsColumn
                        width: Math.max(parent.width, 776)
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
                                        { "label": "Acquired", "width": 92 },
                                        { "label": "Disposed", "width": 92 },
                                        { "label": "Holding", "width": 84 },
                                        { "label": "Sats", "width": 98, "alignRight": true },
                                        { "label": "Cost EUR", "width": 116, "alignRight": true },
                                        { "label": "Proceeds EUR", "width": 126, "alignRight": true },
                                        { "label": "Gain EUR", "width": 112, "alignRight": true }
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
                            model: root.lotRows

                            Rectangle {
                                width: parent.width
                                height: 38
                                color: "transparent"

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
                                        width: 92
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["acquired"] || ""
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 92
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData["disposed"] || ""
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Item {
                                        width: 84
                                        height: parent.height

                                        Rectangle {
                                            anchors.verticalCenter: parent.verticalCenter
                                            width: holdingText.implicitWidth + 12
                                            height: 22
                                            radius: 11
                                            color: "transparent"
                                            border.color: toneColor(modelData["holding_tone"])
                                            border.width: 1

                                            Text {
                                                id: holdingText
                                                anchors.centerIn: parent
                                                text: modelData["holding_label"] || ""
                                                color: toneColor(modelData["holding_tone"])
                                                font.family: theme.monoFont
                                                font.pixelSize: 9
                                                font.weight: Font.DemiBold
                                            }
                                        }
                                    }

                                    Text {
                                        width: 98
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["sats"] || ""
                                        color: theme.ink
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 116
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["cost_label"] || ""
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 126
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["proceeds_label"] || ""
                                        color: theme.ink2
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        width: 112
                                        height: parent.height
                                        verticalAlignment: Text.AlignVCenter
                                        horizontalAlignment: Text.AlignRight
                                        text: modelData["gain_label"] || ""
                                        color: theme.ok
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                        font.weight: Font.DemiBold
                                    }
                                }
                            }
                        }

                        Rectangle {
                            width: parent.width
                            height: 40
                            color: theme.paper

                            Row {
                                anchors.fill: parent
                                anchors.leftMargin: 14
                                anchors.rightMargin: 14
                                spacing: 0

                                Text {
                                    width: 268
                                    height: parent.height
                                    verticalAlignment: Text.AlignVCenter
                                    text: "Total"
                                    color: theme.ink
                                    font.family: theme.monoFont
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    width: 98
                                    height: parent.height
                                    verticalAlignment: Text.AlignVCenter
                                    horizontalAlignment: Text.AlignRight
                                    text: root.totalSats()
                                    color: theme.ink
                                    font.family: theme.monoFont
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    width: 116
                                    height: parent.height
                                    verticalAlignment: Text.AlignVCenter
                                    horizontalAlignment: Text.AlignRight
                                    text: root.summaryValue("Cost basis", "")
                                    color: theme.ink
                                    font.family: theme.monoFont
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    width: 126
                                    height: parent.height
                                    verticalAlignment: Text.AlignVCenter
                                    horizontalAlignment: Text.AlignRight
                                    text: root.summaryValue("Proceeds", "")
                                    color: theme.ink
                                    font.family: theme.monoFont
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    width: 112
                                    height: parent.height
                                    verticalAlignment: Text.AlignVCenter
                                    horizontalAlignment: Text.AlignRight
                                    text: root.summaryValue("Net gain", "")
                                    color: theme.ok
                                    font.family: theme.monoFont
                                    font.pixelSize: 11
                                    font.weight: Font.DemiBold
                                }
                            }
                        }
                    }
                }
            }
        }

        Grid {
            width: parent.width
            columns: root.stackedLayout ? 1 : 3
            columnSpacing: 10
            rowSpacing: 10

            Repeater {
                model: root.exportFormats

                Rectangle {
                    width: root.stackedLayout ? parent.width : (parent.width - 20) / 3
                    height: 96
                    color: modelData["primary"] ? theme.ink : theme.paper2
                    border.color: modelData["primary"] ? theme.ink : theme.line
                    border.width: 1

                    Column {
                        anchors.fill: parent
                        anchors.margins: 16
                        spacing: 3

                        Row {
                            width: parent.width

                            Text {
                                text: modelData["label"] || ""
                                color: modelData["primary"] ? theme.paper2 : theme.ink
                                font.family: theme.serifFont
                                font.pixelSize: 22
                            }

                            Item {
                                width: parent.width - exportText.implicitWidth - parent.children[0].implicitWidth
                                height: 1
                            }

                            Text {
                                id: exportText
                                text: "Read-only"
                                color: modelData["primary"] ? theme.paper2 : theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 10
                                font.bold: true
                            }
                        }

                        Text {
                            text: modelData["summary"] || ""
                            color: modelData["primary"] ? theme.paper2 : theme.ink3
                            font.family: theme.sansFont
                            font.pixelSize: 11
                        }

                        Text {
                            text: modelData["detail"] || ""
                            color: modelData["primary"] ? theme.paper2 : theme.ink2
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                        }
                    }
                }
            }
        }
    }
}
