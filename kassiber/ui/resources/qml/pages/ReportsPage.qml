import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

Item {
    id: root

    property bool hideSensitive: false

    signal requestBack()

    readonly property var reportItems: reportsVM ? reportsVM.items : []
    readonly property var summaryCards: reportsVM ? reportsVM.summaryCards : []
    readonly property var methodOptions: reportsVM ? reportsVM.methodOptions : []
    readonly property var policyRows: reportsVM ? reportsVM.policyRows : []
    readonly property var previewRows: reportsVM ? reportsVM.previewRows : []
    readonly property var exportRows: reportsVM ? reportsVM.exportFormats : []

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
                        text: "Reports"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.capitalization: Font.AllUppercase
                        font.letterSpacing: 1.4
                    }

                    Text {
                        Layout.fillWidth: true
                        text: reportsVM ? reportsVM.statusTitle : "Read-only report surface"
                        color: Design.ink(theme)
                        font.family: Design.serif(theme)
                        font.pixelSize: 28
                        font.weight: Font.Normal
                        font.letterSpacing: -0.2
                        wrapMode: Text.WordWrap
                    }

                    Text {
                        Layout.fillWidth: true
                        text: reportsVM ? reportsVM.statusBody : ""
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: 13
                        wrapMode: Text.WordWrap
                    }
                }

                ActionButton {
                    variant: "ghost"
                    size: "sm"
                    text: "\u2190 Back"
                    onClicked: root.requestBack()
                }
            }

            Card {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                title: "Status"

                ColumnLayout {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: theme.spacingSm

                    Text {
                        Layout.fillWidth: true
                        text: reportsVM ? reportsVM.statusTitle : ""
                        color: root.toneColor(reportsVM ? reportsVM.statusTone : "")
                        font.family: Design.sans()
                        font.pixelSize: theme.fontHeadingSm
                        font.weight: Font.DemiBold
                        wrapMode: Text.WordWrap
                    }

                    Text {
                        Layout.fillWidth: true
                        text: reportsVM ? reportsVM.statusBody : ""
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody
                        wrapMode: Text.WordWrap
                    }
                }
            }

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 4
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                Repeater {
                    model: root.summaryCards

                    delegate: StatTile {
                        Layout.fillWidth: true
                        label: modelData["label"] || ""
                        value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : (modelData["value"] || "")
                        sub: modelData["detail"] || ""
                        valueColor: root.toneColor(modelData["tone"] || "")
                    }
                }
            }

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                Card {
                    Layout.fillWidth: true
                    title: "Available reports"
                    subtitle: "Current desktop routes stay read-only and reflect the active profile policy."

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: theme.spacingSm

                        Repeater {
                            model: root.reportItems

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

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: theme.gridGap

                    Card {
                        Layout.fillWidth: true
                        title: "Lot method"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm - 2

                            Repeater {
                                model: root.methodOptions

                                delegate: RadioRow {
                                    Layout.fillWidth: true
                                    selected: !!modelData["selected"]
                                    label: modelData["label"] || ""
                                    description: modelData["detail"] || ""
                                }
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Policy notes"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm

                            Repeater {
                                model: root.policyRows

                                delegate: Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: policyColumn.implicitHeight + theme.spacingSm + 4
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
                                        id: policyColumn
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.top: parent.top
                                        anchors.margins: theme.spacingSm + 2
                                        spacing: 2

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData["label"] || ""
                                            color: Design.ink3(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontCaption
                                            font.letterSpacing: 1.1
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData["detail"] || ""
                                            color: Design.ink(theme)
                                            font.family: Design.sans()
                                            font.pixelSize: theme.fontBody
                                            wrapMode: Text.WordWrap
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                title: "Recent report inputs"
                subtitle: "Recent non-excluded transactions from the active profile."

                ColumnLayout {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: 0

                    Rectangle {
                        Layout.fillWidth: true
                        visible: root.previewRows.length === 0
                        implicitHeight: emptyText.implicitHeight + theme.cardPadding
                        color: "transparent"

                        Text {
                            id: emptyText
                            anchors.fill: parent
                            anchors.margins: theme.spacingSm + 2
                            text: "No recent transactions are available for the report preview yet."
                            color: Design.ink2(theme)
                            font.family: Design.sans()
                            font.pixelSize: theme.fontBody
                            wrapMode: Text.WordWrap
                        }
                    }

                    Repeater {
                        model: root.previewRows

                        delegate: Rectangle {
                            Layout.fillWidth: true
                            visible: root.previewRows.length > 0
                            implicitHeight: previewRow.implicitHeight + theme.spacingSm + 4
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
                                id: previewRow
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.margins: theme.spacingSm + 2
                                spacing: theme.spacingSm + 4

                                Text {
                                    Layout.preferredWidth: 120
                                    text: modelData["occurred"] || ""
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
                                    wrapMode: Text.WordWrap
                                }

                                TypeBadge {
                                    Layout.alignment: Qt.AlignTop
                                    label: modelData["kind_label"] || ""
                                    tone: modelData["kind_tone"] || ""
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
                                        text: modelData["tags"] || ""
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
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                title: "Export surfaces"
                subtitle: "Presentation of the current report output formats; wiring lands later."

                Flow {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: theme.gridGap

                    Repeater {
                        model: root.exportRows

                        delegate: ExportFormat {
                            width: 220
                            formatName: modelData["label"] || ""
                            subtitle: modelData["summary"] || ""
                            detail: modelData["detail"] || ""
                            primary: !!modelData["primary"]
                        }
                    }
                }
            }
        }
    }
}
