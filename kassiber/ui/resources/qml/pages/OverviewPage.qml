import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

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

    ScrollView {
        anchors.fill: parent
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6

                    Text {
                        Layout.fillWidth: true
                        text: "Snapshot"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.capitalization: Font.AllUppercase
                        font.letterSpacing: 1.4
                    }

                    Text {
                        Layout.fillWidth: true
                        text: dashboardVM.hasData ? "Overview" : dashboardVM.shellTitle
                        color: Design.ink(theme)
                        font.family: Design.serif(theme)
                        font.pixelSize: 28
                        font.weight: Font.Normal
                        font.letterSpacing: -0.2
                        wrapMode: Text.WordWrap
                    }

                    Text {
                        Layout.fillWidth: true
                        text: dashboardVM.hasData
                            ? "Read-only snapshot of the active profile. Values here come from the current local store."
                            : dashboardVM.shellBody
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: 13
                        wrapMode: Text.WordWrap
                    }
                }

                ActionButton {
                    variant: "primary"
                    size: "md"
                    text: "+ Add connection"
                    enabled: connectionsVM ? connectionsVM.canOpenAddConnection : false
                    onClicked: root.requestAddConnection()
                }
            }

            Card {
                visible: !dashboardVM.hasData
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                title: dashboardVM.shellTitle
                subtitle: dashboardVM.shellBody

                ColumnLayout {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: theme.gridGap

                    Repeater {
                        model: root.noticeRows

                        delegate: Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: noticeText.implicitHeight + theme.cardPadding
                            color: Design.paper(theme)
                            border.color: Design.line(theme)
                            border.width: 1

                            Text {
                                id: noticeText
                                anchors.fill: parent
                                anchors.margins: theme.spacingSm + 2
                                text: modelData
                                color: Design.ink2(theme)
                                font.family: Design.sans()
                                font.pixelSize: theme.fontBody
                                wrapMode: Text.WordWrap
                            }
                        }
                    }
                }
            }

            GridLayout {
                visible: dashboardVM.hasData
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 4
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                Repeater {
                    model: root.metricRows

                    delegate: StatTile {
                        Layout.fillWidth: true
                        label: modelData["label"] || ""
                        value: modelData["value"] || ""
                        sub: modelData["tone"] === "warn" ? "Needs attention" : "Current snapshot"
                        valueColor: root.toneColor(modelData["tone"] || "")
                    }
                }
            }

            GridLayout {
                visible: dashboardVM.hasData
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                Card {
                    Layout.fillWidth: true
                    title: "Highlights"
                    subtitle: dashboardVM.projectSummary

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Repeater {
                            model: root.highlightRows

                            delegate: Rectangle {
                                Layout.fillWidth: true
                                implicitHeight: highlightColumn.implicitHeight + theme.cardPadding
                                color: "transparent"

                                Rectangle {
                                    visible: index > 0
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    height: 1
                                    color: Design.line(theme)
                                }

                                ColumnLayout {
                                    id: highlightColumn
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    anchors.margins: theme.spacingSm + 2
                                    spacing: 2

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["title"] || ""
                                        color: Design.ink3(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontCaption
                                        font.letterSpacing: 1.1
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["value"] || ""
                                        color: Design.ink(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontHeadingSm
                                        font.weight: Font.DemiBold
                                        wrapMode: Text.WordWrap
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["body"] || ""
                                        color: Design.ink2(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBodySmall
                                        wrapMode: Text.WordWrap
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    title: "Connections"
                    subtitle: (connectionsVM ? connectionsVM.connectionCount : 0) + " wallet(s)"
                    action: Component {
                        ActionButton {
                            variant: "ghost"
                            size: "sm"
                            text: "+ Add"
                            enabled: connectionsVM ? connectionsVM.canOpenAddConnection : false
                            onClicked: root.requestAddConnection()
                        }
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
                    }
                }
            }

            GridLayout {
                visible: dashboardVM.hasData
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                Card {
                    Layout.fillWidth: true
                    title: "Recent transactions"
                    subtitle: root.transactionRows.length + " loaded into the desktop snapshot"

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Repeater {
                            model: root.visibleTransactionRows

                            delegate: Rectangle {
                                Layout.fillWidth: true
                                implicitHeight: txRow.implicitHeight + theme.spacingSm + 4
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
                                    id: txRow
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    anchors.margins: theme.spacingSm + 2
                                    spacing: theme.spacingSm + 4

                                    Text {
                                        Layout.preferredWidth: 110
                                        text: modelData["occurred_at_label"] || ""
                                        color: Design.ink3(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontCaption
                                        wrapMode: Text.WordWrap
                                    }

                                    TypeBadge {
                                        Layout.alignment: Qt.AlignTop
                                        label: modelData["kind_label"] || ""
                                        tone: modelData["type_tone"] || ""
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 2

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData["wallet"] || ""
                                            color: Design.ink(theme)
                                            font.family: Design.sans()
                                            font.pixelSize: theme.fontBodyStrong
                                            elide: Text.ElideRight
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData["counterparty"] || ""
                                            color: Design.ink2(theme)
                                            font.family: Design.sans()
                                            font.pixelSize: theme.fontBodySmall
                                            elide: Text.ElideRight
                                        }
                                    }

                                    KV {
                                        label: "Amount"
                                        value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : (modelData["amount_label"] || "")
                                        mono: true
                                    }

                                    KV {
                                        label: "Fiat"
                                        value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : (modelData["fiat_label"] || "")
                                        mono: true
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    title: "Reports"
                    subtitle: reportsVM ? reportsVM.statusTitle : ""

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: theme.spacingSm

                        Text {
                            Layout.fillWidth: true
                            text: reportsVM ? reportsVM.statusBody : ""
                            color: Design.ink2(theme)
                            font.family: Design.sans()
                            font.pixelSize: theme.fontBodySmall
                            wrapMode: Text.WordWrap
                        }

                        Repeater {
                            model: root.reportRows

                            delegate: ReportTile {
                                Layout.fillWidth: true
                                title: modelData["label"] || ""
                                subtitle: modelData["summary"] || ""
                                detail: modelData["status"] || ""
                                iconGlyph: (modelData["status_tone"] || "") === "ok" ? "\u2713" : "!"
                            }
                        }
                    }
                }
            }
        }
    }
}
